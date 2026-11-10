//! Router minimum policy and local atomic rollback evidence.
//! The test guard is a model, NOT Titan's production slippage implementation.
#[allow(dead_code)]
mod support;
use solana_pubkey::Pubkey;
use std::path::Path;
#[cfg(feature = "reference-simulation")]
use titan_integration_template::ocelot;
use titan_integration_template::{
    account_caching::AccountsCache,
    ocelot::{OcelotVenue, USDC, codec},
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};

const FIXTURES: [(&str, &str); 4] = [
    (
        "sol3l-465928618.bin.gz",
        "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
    ),
    (
        "sol2s-465928707.bin.gz",
        "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
    ),
    (
        "sol3l-467430913.bin.gz",
        "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
    ),
    (
        "sol2s-467431351.bin.gz",
        "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
    ),
];
async fn fixture(file: &str, key: &str) -> (OcelotVenue, support::Snapshot) {
    let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
    let market = key.parse().unwrap();
    let mut v =
        OcelotVenue::from_account(&market, &cache.get_account(&market).await.unwrap().unwrap())
            .unwrap();
    v.update_state(&cache).await.unwrap();
    (v, cache)
}
fn request(v: &OcelotVenue, mint: bool) -> QuoteRequest {
    let lt = v.get_token_info()[1].pubkey;
    QuoteRequest {
        input_mint: if mint { USDC } else { lt },
        output_mint: if mint { lt } else { USDC },
        amount: if mint {
            10_000_000
        } else {
            10_000_000.min(v.state().unwrap().actual_supply)
        },
        swap_type: SwapType::ExactIn,
    }
}

#[tokio::test]
async fn router_minimum_is_zero_but_standalone_requires_a_positive_floor() {
    let user = Pubkey::new_from_array([211; 32]);
    for (file, key) in FIXTURES {
        let (v, _) = fixture(file, key).await;
        assert!(
            v.get_required_pubkeys_for_update()
                .unwrap()
                .contains(&solana_sdk_ids::sysvar::clock::ID)
        );
        for mint in [true, false] {
            let r = request(&v, mint);
            let quote = v.quote(r.clone()).unwrap();
            let routed = v.generate_swap_instruction(r.clone(), user).unwrap();
            let n = routed.data.len();
            assert_eq!(codec::u64_at(&routed.data, n - 24).unwrap(), 0);
            assert_eq!(codec::i64_at(&routed.data, n - 16).unwrap(), i64::MAX);
            assert_eq!(
                codec::u64_at(&routed.data, n - 8).unwrap(),
                v.state().unwrap().revision
            );
            assert!(v.build_with_minimum(&r, user, 0).is_err());
            let standalone = v
                .build_with_minimum(&r, user, quote.expected_output)
                .unwrap();
            assert_eq!(
                codec::u64_at(&standalone.data, n - 24).unwrap(),
                quote.expected_output
            );
            let mut expected = standalone;
            expected.data[n - 24..n - 16].fill(0);
            assert_eq!(routed, expected, "only the floor differs");
            let mut empty = r.clone();
            empty.amount = 0;
            assert!(v.generate_swap_instruction(empty, user).is_err());
        }
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
#[ignore = "build router SBF binary first; see README"]
async fn adapter_adds_no_deadline_but_program_still_enforces_explicit_expiry() {
    use ocelot::simulation::USER;
    let router = std::fs::read("program-template/target/deploy/titan_v3_venue_template.so")
        .expect("build router first");
    for (file, key) in FIXTURES {
        let (mut v, cache) = fixture(file, key).await;
        v.load_reference_oracle(&cache).await.unwrap();
        // Isolate the Unix deadline from slot-based Phoenix liquidity/oracle
        // expiry. Holding the slot fixed is a synthetic control, not a claim
        // that liquidity survives a real 21-second signing delay. The existing
        // clock/expiry regression separately exercises slot advancement.
        let mut later = v.state().unwrap().clock.clone();
        later.unix_timestamp += 21;
        for mint in [true, false] {
            let r = request(&v, mint);
            let direct = v.generate_swap_instruction(r.clone(), USER).unwrap();
            let bank = v.execution_oracle().unwrap().with_clock(&later);
            let executed = bank
                .execute(direct.clone(), r.input_mint, r.output_mint, r.amount)
                .expect("no adapter deadline at +21 seconds");
            assert!(executed.output > 0);
            let (routed_bank, route) = support::routed(&v, &r, &router, None);
            let routed = routed_bank
                .with_clock(&later)
                .execute(route, r.input_mint, r.output_mint, r.amount)
                .expect("routed instruction also has no 20-second cutoff");
            assert_eq!(routed.output, executed.output);
            let mut expired = direct;
            let n = expired.data.len();
            expired.data[n - 16..n - 8]
                .copy_from_slice(&(v.state().unwrap().clock.unix_timestamp + 20).to_le_bytes());
            let error = bank
                .execute(expired, r.input_mint, r.output_mint, r.amount)
                .expect_err("explicit old deadline must still reject");
            assert!(
                format!("{error:?}").contains("DeadlineExpired"),
                "{error:?}"
            );
        }
    }
    println!(
        "deadline isolation: 16 direct/routed successes after +21 seconds; 8 explicit expired-deadline rejections"
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
#[ignore = "build router and test-only output guard SBF binaries first; see README"]
async fn final_output_guard_reverts_completed_ocelot_and_phoenix_trades() {
    use ocelot::{
        instruction::ata,
        simulation::{USER, token_account},
    };
    use solana_instruction::{AccountMeta, Instruction};
    use std::collections::BTreeSet;
    let guard_id = Pubkey::new_from_array([213; 32]);
    let router = std::fs::read("program-template/target/deploy/titan_v3_venue_template.so")
        .expect("build router first");
    let guard = std::fs::read("program-template/target/deploy/ocelot_test_output_guard.so")
        .expect("build test guard first");
    let modern = |k: Pubkey| modern_address::Address::new_from_array(k.to_bytes());
    let mut successes = 0;
    let mut rollbacks = 0;
    for (file, key) in FIXTURES {
        let (mut v, cache) = fixture(file, key).await;
        v.load_reference_oracle(&cache).await.unwrap();
        for mint in [true, false] {
            let r = request(&v, mint);
            let actual = v.reference_quote(&r).unwrap().output;
            assert!(actual > 1);
            for reserve in [None, Some(0), Some(12_345)] {
                let (oracle, route) = support::routed(&v, &r, &router, reserve);
                // Existing wallet funds must not cover a shortfall in this swap.
                let prior_output = 987_654_321u64;
                let oracle = oracle
                    .with_program(guard_id, &guard)
                    .unwrap()
                    .with_account(
                        ata(USER, r.output_mint),
                        token_account(r.output_mint, USER, prior_output),
                    )
                    .unwrap();
                // Boundary coverage plus an illustrative old quote that is 0.6%
                // above current execution, with user-selected 0.5% / 2% tolerances.
                // This synthetic old quote tests guard policy, not price forecasting.
                let old_quote = (u128::from(actual) * 10_000).div_ceil(9_940) as u64;
                let tight = old_quote - old_quote * 50 / 10_000;
                let loose = old_quote - old_quote * 200 / 10_000;
                assert!(loose < actual && tight > actual);
                for minimum in [actual - 1, actual, actual + 1, tight, loose] {
                    let mut data = prior_output.to_le_bytes().to_vec();
                    data.extend(minimum.to_le_bytes());
                    let check = Instruction {
                        program_id: guard_id,
                        accounts: vec![AccountMeta::new_readonly(ata(USER, r.output_mint), false)],
                        data,
                    };
                    let ixs = [route.clone(), check];
                    let (mut vm, tx, _) = oracle
                        .prepare_transaction(&ixs, r.input_mint, r.amount)
                        .unwrap();
                    let writable: BTreeSet<_> = route
                        .accounts
                        .iter()
                        .filter(|a| a.is_writable)
                        .map(|a| a.pubkey)
                        .collect();
                    let before: Vec<_> = writable
                        .iter()
                        .map(|key| (*key, vm.get_account(&modern(*key))))
                        .collect();
                    let result = vm.send_transaction(tx);
                    if minimum <= actual {
                        let meta = result.expect("final minimum satisfied");
                        assert!(
                            meta.logs
                                .iter()
                                .any(|l| l.contains("test-only route output:"))
                        );
                        assert_eq!(
                            codec::u64_at(
                                &vm.get_account(&modern(ata(USER, r.output_mint)))
                                    .unwrap()
                                    .data,
                                64
                            )
                            .unwrap(),
                            prior_output + actual
                        );
                        assert_eq!(
                            codec::u64_at(
                                &vm.get_account(&modern(ata(USER, r.input_mint)))
                                    .unwrap()
                                    .data,
                                64
                            )
                            .unwrap(),
                            0
                        );
                        assert!(
                            before.iter().any(|(key, a)| a
                                .as_ref()
                                .is_some_and(|a| a.owner == modern(ocelot::PHOENIX))
                                && vm.get_account(&modern(*key)) != *a),
                            "successful control must change Phoenix state"
                        );
                        successes += 1;
                    } else {
                        let failure = result.expect_err("below user minimum must reject");
                        assert_eq!(
                            format!("{:?}", failure.err),
                            "InstructionError(1, Custom(1))",
                            "failure must come from the final guard: {:?}",
                            failure.meta.logs
                        );
                        for program in [support::ROUTER, ocelot::PROGRAM_ID, ocelot::PHOENIX] {
                            assert!(
                                failure
                                    .meta
                                    .logs
                                    .iter()
                                    .any(|l| l == &format!("Program {program} success")),
                                "trade must have completed before the final guard"
                            );
                        }
                        assert!(
                            failure
                                .meta
                                .logs
                                .iter()
                                .any(|l| l
                                    .contains(&format!("received={actual}, minimum={minimum}")))
                        );
                        for (key, mut expected) in before {
                            if key == USER {
                                expected.as_mut().unwrap().lamports -= failure.meta.fee;
                            }
                            assert_eq!(
                                vm.get_account(&modern(key)),
                                expected,
                                "rollback differs for {key}: {file}, mint={mint}, reserve={reserve:?}"
                            );
                        }
                        rollbacks += 1;
                    }
                }
            }
        }
    }
    assert_eq!((successes, rollbacks), (72, 48));
    println!(
        "route minimum model: {successes} successful transactions; {rollbacks} complete rollbacks after Ocelot/Phoenix success, all writable accounts restored except payer fee"
    );
}

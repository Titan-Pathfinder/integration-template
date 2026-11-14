#![allow(clippy::result_large_err)]
//! Tests exercise the bundled provider through the actual venue interface.
#[allow(dead_code)]
mod support;
use solana_pubkey::Pubkey;
use std::path::Path;
use titan_integration_template::{
    ocelot::{self, OcelotVenue, USDC},
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

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
async fn load(cache: &support::Snapshot, key: Pubkey) -> OcelotVenue {
    let mut v = OcelotVenue::from_account(&key, cache.0[&key].as_ref().unwrap()).unwrap();
    v.update_state(cache).await.unwrap();
    v
}
async fn fixture(i: usize) -> (OcelotVenue, support::Snapshot) {
    let (file, key) = FIXTURES[i];
    let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
    (load(&cache, key.parse().unwrap()).await, cache)
}
fn request(v: &OcelotVenue, mint: bool, amount: u64) -> QuoteRequest {
    let lt = v.state().unwrap().market.mint;
    QuoteRequest {
        input_mint: if mint { USDC } else { lt },
        output_mint: if mint { lt } else { USDC },
        amount,
        swap_type: SwapType::ExactIn,
    }
}
fn put64(d: &mut [u8], o: usize, n: u64) {
    d[o..o + 8].copy_from_slice(&n.to_le_bytes());
}
fn put32(d: &mut [u8], o: usize, n: u32) {
    d[o..o + 4].copy_from_slice(&n.to_le_bytes());
}

fn asset_entry(cache: &support::Snapshot, asset: u64) -> usize {
    let d = &cache.0[&ocelot::PERP_MAP].as_ref().unwrap().data;
    let count = u32::from_le_bytes(d[32..36].try_into().unwrap()) as usize;
    (0..count)
        .map(|i| 64 + i * 1584)
        .find(|&o| {
            let id = u16::from_le_bytes(d[o + 944..o + 946].try_into().unwrap()) as u64
                | ((u16::from_le_bytes(d[o + 948..o + 950].try_into().unwrap()) as u64) << 16);
            d[o + 1450] == 0 && id == asset
        })
        .unwrap()
}

#[tokio::test]
async fn unreviewed_oracle_policies_invalidate_prepared_quotes() {
    for i in 0..FIXTURES.len() {
        let (v, cache) = fixture(i).await;
        let o = asset_entry(&cache, v.state().unwrap().config.asset);
        // Order/withdrawal rules, including a previously unused book condition;
        // both bytes of the radius, quorum and both hard-stale multipliers.
        for (offset, value) in [
            (352 + 2 * 32 + 2, 2),
            (352 + 4 * 32 + 2, 2),
            (352 + 2 * 32 + 3, 1),
            (352 + 2 * 32 + 2 * 8 + 2, 1),
            (808, 0),
            (809, 0),
            (810, 2),
            (811, 1),
            (812, 1),
        ] {
            let mut changed = support::Snapshot(cache.0.clone());
            let d = &mut changed
                .0
                .get_mut(&ocelot::PERP_MAP)
                .unwrap()
                .as_mut()
                .unwrap()
                .data;
            assert_ne!(d[o + offset], value);
            d[o + offset] = value;
            let mut candidate = v.clone();
            assert!(candidate.update_state(&changed).await.is_err());
            assert!(!candidate.initialized());
            assert!(candidate.state().is_err());
            for mint in [true, false] {
                assert!(candidate.quote(request(&v, mint, 5_000_000)).is_err());
            }
        }
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn divergence_quorum_and_validity_rules_match_execution_rejections() {
    use ocelot::simulation::USER;
    let mut accepted = 0;
    let mut divergent_rejections = 0;
    let mut changed_rule_rejections = 0;
    for (i, _) in FIXTURES.iter().enumerate() {
        let (mut v, cache) = fixture(i).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let s = v.state().unwrap();
        let o = asset_entry(&cache, s.config.asset);
        for score in [1, 255] {
            // One/two flagged responses must not poison the remaining quorum
            // or the independent book component. Five flagged responses must.
            for flagged in [1, 2, 5] {
                let mut changed = support::Snapshot(cache.0.clone());
                let a = changed
                    .0
                    .get_mut(&ocelot::PERP_MAP)
                    .unwrap()
                    .as_mut()
                    .unwrap();
                for k in 0..5 {
                    a.data[o + 640 + 40 * k] = if k < flagged { score } else { 0 };
                    for start in [32, 168] {
                        put64(&mut a.data, o + start + 16 * k, s.clock.slot);
                    }
                }
                let oracle = v
                    .execution_oracle()
                    .unwrap()
                    .with_account(ocelot::PERP_MAP, a.clone())
                    .unwrap();
                let mut candidate = v.clone();
                let refreshed = candidate.update_state(&changed).await;
                assert_eq!(refreshed.is_ok(), flagged < 5);
                for mint in [true, false] {
                    let r = request(&v, mint, 5_000_000);
                    let executed = oracle.execute(
                        v.build_with_minimum(&r, USER, 1).unwrap(),
                        r.input_mint,
                        r.output_mint,
                        r.amount,
                    );
                    if flagged == 5 {
                        assert!(!candidate.initialized());
                        assert!(candidate.quote(r).is_err());
                        assert!(
                            executed.is_err(),
                            "all divergent: {} mint={mint}",
                            FIXTURES[i].0
                        );
                        divergent_rejections += 1;
                    } else {
                        let q = candidate.quote(r).unwrap();
                        assert!(q.expected_output <= executed.unwrap().output);
                        accepted += 1;
                    }
                }
            }
        }
        // The review's rule mutation can reject execution without changing the
        // mark or quotes. The production guard above rejects every policy edit,
        // even when a particular captured transaction would still execute.
        let mut a = cache.0[&ocelot::PERP_MAP].clone().unwrap();
        a.data[o + 352 + 2 * 32 + 2] = 2;
        let oracle = v
            .execution_oracle()
            .unwrap()
            .with_account(ocelot::PERP_MAP, a)
            .unwrap();
        for mint in [true, false] {
            let r = request(&v, mint, 5_000_000);
            if oracle
                .execute(
                    v.build_with_minimum(&r, USER, 1).unwrap(),
                    r.input_mint,
                    r.output_mint,
                    r.amount,
                )
                .is_err()
            {
                changed_rule_rejections += 1;
            }
        }
    }
    assert_eq!(accepted, 32);
    assert_eq!(divergent_rejections, 16);
    assert!(changed_rule_rejections > 0);
    eprintln!(
        "oracle regression: {accepted} accepted executions, {divergent_rejections} all-divergent execution rejections, {changed_rule_rejections} changed-rule execution rejections"
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn fresh_divergent_responses_cannot_replace_a_stale_eligible_oracle() {
    for i in [2, 3] {
        let (mut v, cache) = fixture(i).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let s = v.state().unwrap();
        let o = asset_entry(&cache, s.config.asset);
        for extra_age in [10, 100] {
            let mut changed = support::Snapshot(cache.0.clone());
            let a = changed
                .0
                .get_mut(&ocelot::PERP_MAP)
                .unwrap()
                .as_mut()
                .unwrap();
            for k in 0..5 {
                a.data[o + 640 + 40 * k] = u8::from(k != 0);
                for (start, threshold) in [(32, 120), (168, 256)] {
                    let age = ocelot::codec::u64_at(&a.data, o + threshold).unwrap() + extra_age;
                    put64(
                        &mut a.data,
                        o + start + 16 * k,
                        if k == 0 {
                            s.clock.slot - age
                        } else {
                            s.clock.slot
                        },
                    );
                }
            }
            let oracle = v
                .execution_oracle()
                .unwrap()
                .with_account(ocelot::PERP_MAP, a.clone())
                .unwrap();
            let mut candidate = v.clone();
            assert!(candidate.update_state(&changed).await.is_err());
            assert!(!candidate.initialized());
            let r = request(&v, false, 5_000_000);
            assert!(candidate.quote(r.clone()).is_err());
            assert!(
                oracle
                    .execute(
                        v.build_with_minimum(&r, ocelot::simulation::USER, 1)
                            .unwrap(),
                        r.input_mint,
                        r.output_mint,
                        r.amount
                    )
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn native_provider_initializes_and_quotes_without_io_or_allocations() {
    for (i, _) in FIXTURES.iter().enumerate() {
        let (v, _) = fixture(i).await;
        assert!(v.initialized());
        for mint in [true, false] {
            let r = request(&v, mint, 5_000_000);
            let q = assert_no_alloc::assert_no_alloc(|| v.quote(r.clone()));
            eprintln!("{} mint={mint} {q:?}", FIXTURES[i].0);
            let q = q.unwrap();
            assert!(q.expected_output > 0);
            assert!(q.price.is_finite() && q.price > 0.0);
            assert!(assert_no_alloc::assert_no_alloc(|| v.quote(request(&v, mint, 1))).is_err());
            assert_eq!(
                assert_no_alloc::assert_no_alloc(|| v.quote(request(&v, mint, 0)))
                    .unwrap()
                    .expected_output,
                0
            );
        }
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn zero_weight_oracles_still_require_freshness() {
    use ocelot::simulation::USER;
    let mut accepted = 0;
    let mut rejected = 0;
    let mut conservative_declines = 0;
    for (i, (file, _)) in FIXTURES.iter().enumerate() {
        let (mut v, cache) = fixture(i).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let s = v.state().unwrap();
        let o = asset_entry(&cache, s.config.asset);
        for (start, weight, threshold, count) in
            [(32, 112, 120, 5), (168, 248, 256, 5), (264, 280, 288, 1)]
        {
            let original = cache.0[&ocelot::PERP_MAP].as_ref().unwrap();
            let threshold = ocelot::codec::u64_at(&original.data, o + threshold).unwrap();
            for age in [0, threshold + 1, 500] {
                let mut changed = support::Snapshot(cache.0.clone());
                let a = changed
                    .0
                    .get_mut(&ocelot::PERP_MAP)
                    .unwrap()
                    .as_mut()
                    .unwrap();
                put64(&mut a.data, o + weight, 0);
                for k in 0..count {
                    put64(&mut a.data, o + start + 16 * k, s.clock.slot - age);
                }
                let oracle = v
                    .execution_oracle()
                    .unwrap()
                    .with_account(ocelot::PERP_MAP, a.clone())
                    .unwrap();
                let mut candidate = v.clone();
                let refreshed = candidate.update_state(&changed).await;
                // Weight affects mark composition, not spot/perp validity. The
                // zero-weight book is an executable exception in this policy.
                let valid = count == 1 || age == 0;
                assert_eq!(refreshed.is_ok(), valid, "{file}: start={start} age={age}");
                assert_eq!(candidate.initialized(), valid);
                if !valid {
                    assert!(candidate.state().is_err());
                }
                for mint in [true, false] {
                    for amount in [1_000_000, 5_000_000] {
                        let r = request(&v, mint, amount);
                        let executed = oracle.execute(
                            v.build_with_minimum(&r, USER, 1).unwrap(),
                            r.input_mint,
                            r.output_mint,
                            r.amount,
                        );
                        let quoted = candidate.quote(r);
                        if valid {
                            assert!(quoted.unwrap().expected_output <= executed.unwrap().output);
                            accepted += 1;
                        } else {
                            assert!(quoted.is_err());
                            // A small soft-stale penalty may leave some trades
                            // executable; declining them is intentional. Severe
                            // staleness and the review's recent redeem cases fail.
                            if age == 500 || (i >= 2 && !mint) {
                                assert!(
                                    executed.is_err(),
                                    "{file}: start={start} age={age} mint={mint}"
                                );
                            }
                            if executed.is_err() {
                                rejected += 1;
                            } else {
                                conservative_declines += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(accepted, 80);
    assert_eq!(rejected + conservative_declines, 64);
    eprintln!(
        "zero-weight regression: {accepted} accepted executions, {rejected} execution rejections, {conservative_declines} deliberate conservative declines"
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn complete_native_quotes_do_not_overquote_captured_execution() {
    for (i, _) in FIXTURES.iter().enumerate() {
        let (mut v, cache) = fixture(i).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let mut successes = 0;
        let mut declines = 0;
        let mut worst_bps = 0.0f64;
        for mint in [true, false] {
            let s = v.state().unwrap();
            let mut amounts = vec![
                1,
                500_000,
                1_000_000,
                5_000_000,
                10_000_000,
                50_000_000,
                100_000_000,
                500_000_000,
                1_000_000_000,
            ];
            // Inputs immediately around lot steps, including deeper fills.
            for lots in [1u64, 2, 3, 10, 100, 1000, 3000] {
                let boundary = if mint {
                    let net =
                        (lots as u128 * s.mark.quote_per_lot).div_ceil(s.market.leverage as u128);
                    (net * 10_000).div_ceil(10_000 - s.market.mint_fee_bps as u128) as u64
                } else {
                    (lots as u128 * s.market.shares as u128)
                        .div_ceil(s.position.base.unsigned_abs() as u128) as u64
                };
                amounts.extend([boundary.saturating_sub(1), boundary, boundary + 1]);
            }
            if !mint {
                amounts.push(s.actual_supply);
            }
            amounts.sort_unstable();
            amounts.dedup();
            for amount in amounts {
                let r = request(&v, mint, amount);
                let Ok(q) = v.quote(r.clone()) else {
                    assert!(
                        v.reference_quote(&r).is_err(),
                        "{} mint={mint} input={amount}: unclassified false decline",
                        FIXTURES[i].0
                    );
                    declines += 1;
                    continue;
                };
                let execution = v.reference_quote(&r).unwrap_or_else(|e| {
                    panic!(
                        "{} mint={mint} input={amount} native={} falsely accepted: {e:?}",
                        FIXTURES[i].0, q.expected_output
                    )
                });
                assert!(
                    q.expected_output <= execution.output,
                    "{} mint={mint} input={amount} native={} actual={}",
                    FIXTURES[i].0,
                    q.expected_output,
                    execution.output
                );
                worst_bps = worst_bps.max(
                    (execution.output - q.expected_output) as f64 * 10_000.0
                        / execution.output as f64,
                );
                successes += 1;
            }
        }
        assert!(successes >= 10, "insufficient accepted coverage");
        eprintln!(
            "native {}: accepted={successes}, declined={declines}, worst underquote={worst_bps:.6} bps",
            FIXTURES[i].0
        );
    }
}

#[tokio::test]
async fn native_refresh_fees_capabilities_liquidity_and_withdrawals() {
    let (mut v, mut cache) = fixture(2).await;
    let original = cache.0.clone();
    let s = v.state().unwrap().clone();
    let mint = request(&v, true, 10_000_000);
    let redeem = request(&v, false, 5_000_000);
    let before = v.quote(mint.clone()).unwrap().expected_output;
    cache
        .0
        .get_mut(&s.config.trader)
        .unwrap()
        .as_mut()
        .unwrap()
        .data[103] = 5;
    v.update_state(&cache).await.unwrap();
    assert!(
        v.quote(mint.clone()).unwrap().expected_output < before,
        "read the trader fee override on refresh"
    );
    cache.0 = original.clone();
    let d = &mut cache
        .0
        .get_mut(&s.exchange.indices[0])
        .unwrap()
        .as_mut()
        .unwrap()
        .data;
    for node in d[96..].chunks_exact_mut(64) {
        put64(node, 48, 0);
    }
    v.update_state(&cache).await.unwrap();
    assert!(
        v.quote(mint.clone()).is_err(),
        "declared depth alone cannot imply fill availability"
    );
    cache.0 = original.clone();
    put32(
        &mut cache
            .0
            .get_mut(&s.config.trader)
            .unwrap()
            .as_mut()
            .unwrap()
            .data,
        96,
        54,
    );
    v.update_state(&cache).await.unwrap();
    assert!(v.quote(mint.clone()).is_err());
    assert!(
        v.quote(redeem.clone()).is_ok(),
        "risk-reducing exit remains possible"
    );
    cache.0 = original.clone();
    let d = &mut cache
        .0
        .get_mut(&s.exchange.withdraw_queue)
        .unwrap()
        .as_mut()
        .unwrap()
        .data;
    put64(d, 32, 0);
    put64(d, 40, 0);
    v.update_state(&cache).await.unwrap();
    assert!(v.quote(redeem.clone()).is_err());
    assert!(v.quote(mint.clone()).is_ok());
    cache.0 = original.clone();
    put64(
        &mut cache
            .0
            .get_mut(&s.exchange.vault)
            .unwrap()
            .as_mut()
            .unwrap()
            .data,
        64,
        0,
    );
    v.update_state(&cache).await.unwrap();
    assert!(v.quote(redeem.clone()).is_err());
    cache.0 = original.clone();
    let spline =
        Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &ocelot::PHOENIX).0;
    let d = &mut cache.0.get_mut(&spline).unwrap().as_mut().unwrap().data;
    let n = u32::from_le_bytes(d[72..76].try_into().unwrap()) as usize;
    for j in 0..n {
        put32(d, 112 + j * 1336 + 1100, 10_000);
    }
    v.update_state(&cache).await.unwrap();
    assert!(v.quote(mint.clone()).is_err());
    assert!(v.quote(redeem).is_err());
    cache.0 = original;
    cache.0.insert(spline, None);
    assert!(v.update_state(&cache).await.is_err());
    assert!(!v.initialized());
    assert!(v.state().is_err());
    assert!(assert_no_alloc::assert_no_alloc(|| v.quote(mint)).is_err());
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn native_fee_overrides_and_budget_mutations_against_execution() {
    let (v, mut cache) = fixture(2).await;
    let original = cache.0.clone();
    let s = v.state().unwrap().clone();
    for (fee, multiplier) in [(351, -5i8), (351, 5), (0, 0), (350, -10), (1000, 20)] {
        cache.0 = original.clone();
        put32(
            &mut cache
                .0
                .get_mut(&s.config.book)
                .unwrap()
                .as_mut()
                .unwrap()
                .data,
            96,
            fee,
        );
        cache
            .0
            .get_mut(&s.config.trader)
            .unwrap()
            .as_mut()
            .unwrap()
            .data[103] = multiplier as u8;
        let mut v = load(&cache, s.config.market).await;
        v.load_reference_oracle(&cache).await.unwrap();
        for mint in [true, false] {
            for amount in [500_000, 10_000_000, 500_000_000] {
                let r = request(&v, mint, amount);
                let q = v.quote(r.clone()).unwrap();
                let actual = v.reference_quote(&r).unwrap();
                assert!(
                    q.expected_output <= actual.output,
                    "fee={fee} multiplier={multiplier} mint={mint} amount={amount}: {} > {}",
                    q.expected_output,
                    actual.output
                );
            }
        }
    }
    cache.0 = original.clone();
    // Direct withdrawals use the immediate path. Queue fees do not change a
    // successful immediate withdrawal; exhaustion must not turn it into a quote.
    put64(
        &mut cache
            .0
            .get_mut(&s.exchange.withdraw_queue)
            .unwrap()
            .as_mut()
            .unwrap()
            .data,
        64,
        100_000_000,
    );
    let mut v = load(&cache, s.config.market).await;
    v.load_reference_oracle(&cache).await.unwrap();
    let r = request(&v, false, 5_000_000);
    assert!(v.quote(r.clone()).unwrap().expected_output <= v.reference_quote(&r).unwrap().output);
    let d = &mut cache
        .0
        .get_mut(&s.exchange.withdraw_queue)
        .unwrap()
        .as_mut()
        .unwrap()
        .data;
    put64(d, 32, 0);
    put64(d, 40, 0);
    v.update_state(&cache).await.unwrap();
    v.load_reference_oracle(&cache).await.unwrap();
    assert!(v.quote(r.clone()).is_err());
    assert!(v.reference_quote(&r).is_err());

    cache.0 = original;
    let spline =
        Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &ocelot::PHOENIX).0;
    let d = &mut cache.0.get_mut(&spline).unwrap().as_mut().unwrap().data;
    let n = u32::from_le_bytes(d[72..76].try_into().unwrap()) as usize;
    for j in 0..n {
        let o = 112 + j * 1336;
        put32(d, o + 1096, 1);
        put32(d, o + 1104, 0);
        put32(d, o + 1108, 0);
    }
    let mut v = load(&cache, s.config.market).await;
    v.load_reference_oracle(&cache).await.unwrap();
    let mut accepted = 0;
    for mint in [true, false] {
        for amount in [1_000_000, 5_000_000, 100_000_000] {
            let r = request(&v, mint, amount);
            if let Ok(q) = v.quote(r.clone()) {
                let actual = v.reference_quote(&r).unwrap();
                assert!(
                    q.expected_output <= actual.output,
                    "reduce-only maker caps overquoted"
                );
                accepted += 1;
            }
        }
    }
    assert!(
        accepted > 0,
        "maker caps still allow supported risk-reducing liquidity"
    );
}

#[tokio::test]
async fn native_rejects_unreviewed_or_malformed_cached_state() {
    let (mut v, mut cache) = fixture(2).await;
    let original = cache.0.clone();
    let s = v.state().unwrap().clone();
    let r = request(&v, true, 5_000_000);
    let ember =
        Pubkey::find_program_address(&[ocelot::PHOENIX.as_ref(), b"state"], &ocelot::EMBER).0;
    for (key, offset, value) in [
        (ember, 8, 42),
        (s.config.trader, 96, 63),
        (s.exchange.indices[0], 80, 255),
        (ocelot::PHOENIX_GLOBAL, 504, 0),
    ] {
        cache.0 = original.clone();
        cache.0.get_mut(&key).unwrap().as_mut().unwrap().data[offset] = value;
        assert!(
            v.update_state(&cache).await.is_err(),
            "mutation {key} at {offset}"
        );
        assert!(!v.initialized());
        assert!(v.quote(r.clone()).is_err());
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn better_execution_must_fit_real_withdrawal_capacity() {
    let (v, mut cache) = fixture(2).await;
    let pool = v.pool_id;
    // Remove issuer fees/liabilities so output equals the actual gross payout.
    let d = &mut cache.0.get_mut(&pool).unwrap().as_mut().unwrap().data;
    d[239..243].fill(0);
    put64(d, 281, 0);
    put64(d, 309, 0);
    let mut v = load(&cache, pool).await;
    v.load_reference_oracle(&cache).await.unwrap();
    let r = request(&v, false, 5_000_000);
    let estimated = v.quote(r.clone()).unwrap().expected_output;
    let actual = v.reference_quote(&r).unwrap().output;
    assert!(
        actual > estimated + 1,
        "fixture must exercise a strictly conservative estimate"
    );
    let s = v.state().unwrap();
    let cash = titan_integration_template::ocelot::quote::Limits::load(s)
        .unwrap()
        .cash;
    let cash_claim = (cash as u128 * r.amount as u128 / s.market.shares as u128) as u64;
    let d = &mut cache
        .0
        .get_mut(&s.exchange.withdraw_queue)
        .unwrap()
        .as_mut()
        .unwrap()
        .data;
    put64(d, 32, estimated + (actual - estimated) / 2 - cash_claim);
    put64(d, 40, 0);
    v.update_state(&cache).await.unwrap();
    v.load_reference_oracle(&cache).await.unwrap();
    assert!(
        v.quote(r.clone()).is_err(),
        "capacity must cover actual payout, not just lower estimate"
    );
    assert!(
        v.reference_quote(&r).is_err(),
        "actual withdrawal exhausts the constrained budget"
    );
}

#[tokio::test]
#[ignore = "run with --release; measures complete OcelotVenue::quote including native Phoenix"]
async fn benchmark_complete_native_quote() {
    use std::{hint::black_box, time::Instant};
    assert!(
        !black_box(cfg!(debug_assertions)),
        "run this benchmark with --release"
    );
    let (mut v, cache) = fixture(2).await;
    let reqs = [request(&v, true, 10_000_000), request(&v, false, 5_000_000)];
    for r in &reqs {
        v.quote(r.clone()).unwrap();
    }
    let mut batches = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        for i in 0..10_000 {
            black_box(v.quote(black_box(reqs[i % 2].clone())).unwrap());
        }
        batches.push(start.elapsed().as_nanos() as f64 / 10_000.0);
    }
    batches.sort_by(f64::total_cmp);
    eprintln!(
        "complete OcelotVenue::quote: 1,000,000 calls, batch-mean p50={:.1}ns p99={:.1}ns max={:.1}ns (nearest-rank; refresh excluded)",
        batches[49], batches[98], batches[99]
    );
    assert!(
        batches[99] < 1_000.0,
        "native quoting exceeded Titan's 1us heuristic on this host"
    );
    let mut preparation = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        black_box(ocelot::phoenix::NativePhoenix::prepare(v.state().unwrap()).unwrap());
        preparation.push(start.elapsed().as_nanos() as f64 / 1_000.0);
    }
    preparation.sort_by(f64::total_cmp);
    eprintln!(
        "native preparation: 100 calls p50={:.1}us p99={:.1}us max={:.1}us (nearest-rank; cached-state decoding, excludes cache fetch/copy)",
        preparation[49], preparation[98], preparation[99]
    );
    let mut refresh = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        v.update_state(&cache).await.unwrap();
        refresh.push(start.elapsed().as_nanos() as f64 / 1_000.0);
    }
    refresh.sort_by(f64::total_cmp);
    eprintln!(
        "complete cached update_state: 100 calls p50={:.1}us p99={:.1}us max={:.1}us (nearest-rank; in-memory Snapshot cache, no network)",
        refresh[49], refresh[98], refresh[99]
    );
    // Fan-out cost only: instances repeat the four captured SOL markets, not
    // 26 distinct live markets. Shared-decoder optimizations are not assumed.
    let mut snapshots = Vec::new();
    for i in 0..FIXTURES.len() {
        snapshots.push(fixture(i).await);
    }
    for count in [4usize, 26] {
        let mut venues: Vec<_> = (0..count).map(|i| snapshots[i % 4].0.clone()).collect();
        let mut elapsed = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            for (i, venue) in venues.iter_mut().enumerate() {
                venue.update_state(&snapshots[i % 4].1).await.unwrap();
            }
            elapsed.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        elapsed.sort_by(f64::total_cmp);
        eprintln!(
            "cached refresh fan-out: {count} fixture instances, 100 cycles p50={:.3}ms p99={:.3}ms max={:.3}ms (nearest-rank; sequential; excludes network)",
            elapsed[49], elapsed[98], elapsed[99]
        );
    }
    let mut individual = Vec::with_capacity(100_000);
    let mut timer = Vec::with_capacity(100_000);
    for i in 0..100_000 {
        let start = Instant::now();
        black_box(v.quote(black_box(reqs[i % 2].clone())).unwrap());
        individual.push(start.elapsed().as_nanos());
        let start = Instant::now();
        timer.push(black_box(start.elapsed().as_nanos()));
    }
    individual.sort_unstable();
    timer.sort_unstable();
    eprintln!(
        "individual complete quotes: 100,000 calls p50={}ns p99={}ns p99.9={}ns max={}ns; timer-only p50={}ns p99={}ns (nearest-rank; includes timer overhead and scheduling)",
        individual[49_999],
        individual[98_999],
        individual[99_899],
        individual[99_999],
        timer[49_999],
        timer[98_999]
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn finance_cash_redemptions_match_execution() {
    for i in 0..FIXTURES.len() {
        let (v, mut cache) = fixture(i).await;
        let s = v.state().unwrap();
        let cash = s.gross / 5;
        let pool = v.pool_id;
        let d = &mut cache.0.get_mut(&pool).unwrap().as_mut().unwrap().data;
        let flags = ocelot::codec::u64_at(d, 317).unwrap();
        put64(d, 317, flags | 1024);
        let finance = ocelot::instruction::pda(b"finance_v1", pool);
        let d = &mut cache.0.get_mut(&finance).unwrap().as_mut().unwrap().data;
        d[42] = 1;
        put64(d, 51, cash);
        let vault = ocelot::instruction::pda(b"market_cash_v1", pool);
        put64(
            &mut cache.0.get_mut(&vault).unwrap().as_mut().unwrap().data,
            64,
            cash,
        );
        let mut v = load(&cache, pool).await;
        v.load_reference_oracle(&cache).await.unwrap();
        for amount in [1_000_000, v.state().unwrap().actual_supply] {
            let r = request(&v, false, amount);
            let q = v.quote(r.clone()).unwrap();
            let ix = v
                .generate_swap_instruction(r.clone(), ocelot::simulation::USER)
                .unwrap();
            let (execution, accounts) = v
                .execution_oracle()
                .unwrap()
                .execute_with_accounts(ix, r.input_mint, r.output_mint, r.amount)
                .unwrap();
            assert!(q.expected_output <= execution.output);
            let cash_paid = cash - ocelot::codec::u64_at(&accounts[&vault].data, 64).unwrap();
            assert_eq!(
                cash_paid,
                (cash as u128 * amount as u128 / v.state().unwrap().market.shares as u128) as u64
            );
            assert!(cash_paid > 0, "must exercise actual cash transfer");
        }
        put64(
            &mut cache.0.get_mut(&vault).unwrap().as_mut().unwrap().data,
            64,
            cash - 1,
        );
        assert!(
            v.update_state(&cache).await.is_err(),
            "cash must be backed by custody"
        );
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn full_exit_capacity_covers_better_execution() {
    for i in 0..FIXTURES.len() {
        let (v, mut cache) = fixture(i).await;
        let pool = v.pool_id;
        let d = &mut cache.0.get_mut(&pool).unwrap().as_mut().unwrap().data;
        d[239..243].fill(0);
        put64(d, 281, 0);
        put64(d, 309, 0);
        let mut v = load(&cache, pool).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let r = request(&v, false, v.state().unwrap().actual_supply);
        let estimated = v.quote(r.clone()).unwrap().expected_output;
        let actual = v.reference_quote(&r).unwrap().output;
        assert!(actual > estimated + 1);
        let queue = v.state().unwrap().exchange.withdraw_queue;
        let original = cache.0.clone();
        for budget in [estimated + (actual - estimated) / 2, actual] {
            cache.0 = original.clone();
            let d = &mut cache.0.get_mut(&queue).unwrap().as_mut().unwrap().data;
            put64(d, 32, budget);
            put64(d, 40, 0);
            v.update_state(&cache).await.unwrap();
            v.load_reference_oracle(&cache).await.unwrap();
            let native = v.quote(r.clone());
            let executed = v.reference_quote(&r);
            if budget < actual {
                assert!(
                    native.is_err(),
                    "underestimate cannot determine withdrawal capacity"
                );
                assert!(executed.is_err(), "actual full exit exceeds this budget");
            } else {
                assert_eq!(executed.unwrap().output, actual);
                // The provider reserves a zero-fee upper bound, so it may decline
                // even when exactly the actual budget would execute.
                if let Ok(q) = native {
                    assert!(q.expected_output <= actual);
                }
            }
        }
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn original_transactions_keep_output_and_deadline_guards_as_liquidity_expires() {
    use ocelot::simulation::USER;
    let mut deteriorated = 0;
    let mut rejected = 0;
    let mut successful = 0;
    for i in 0..FIXTURES.len() {
        let (mut v, cache) = fixture(i).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let mut originals = vec![];
        for mint in [true, false] {
            for amount in [1_000_000, 10_000_000, 1_000_000_000] {
                let r = request(&v, mint, amount);
                if let Ok(q) = v.quote(r.clone()) {
                    // Preserve the original explicit 1% floor and 20-second
                    // deadline in this historical regression. Titan now owns
                    // output protection and expiry for generated route legs.
                    let minimum = q
                        .expected_output
                        .saturating_sub(q.expected_output / 100)
                        .max(1);
                    let mut ix = v.build_with_minimum(&r, USER, minimum).unwrap();
                    let n = ix.data.len();
                    ix.data[n - 16..n - 8].copy_from_slice(
                        &(v.state().unwrap().clock.unix_timestamp + 20).to_le_bytes(),
                    );
                    let floor = ocelot::codec::u64_at(&ix.data, ix.data.len() - 24).unwrap();
                    originals.push((r, q.expected_output, floor, ix));
                }
            }
        }
        for k in [0u64, 4, 5, 6, 12, 19, 51] {
            let mut later = support::Snapshot(cache.0.clone());
            let mut clock = v.state().unwrap().clock.clone();
            clock.slot += k;
            clock.unix_timestamp += (k * 2).div_ceil(5) as i64;
            later
                .0
                .get_mut(&solana_sdk_ids::sysvar::clock::ID)
                .unwrap()
                .as_mut()
                .unwrap()
                .data = bincode::serialize(&clock).unwrap();
            let mut refreshed = v.clone();
            let prepared = refreshed.update_state(&later).await;
            if k == 51 {
                assert!(
                    prepared.is_err(),
                    "stale oracle components must invalidate native quotes"
                );
                assert!(!refreshed.initialized());
            } else {
                prepared.unwrap();
            }
            let execution_bank = v.execution_oracle().unwrap().with_clock(&clock);
            for (r, quoted, floor, ix) in &originals {
                let exec =
                    execution_bank.execute(ix.clone(), r.input_mint, r.output_mint, r.amount);
                if k == 51 {
                    assert!(exec.is_err(), "original deadline remains in force");
                    // Keep maker prices live to isolate the deadline from a
                    // coincident liquidity failure. Only extending the deadline
                    // must make this same encoded order executable again.
                    let spline_key = Pubkey::find_program_address(
                        &[b"spline", v.state().unwrap().config.book.as_ref()],
                        &ocelot::PHOENIX,
                    )
                    .0;
                    let mut spline = cache.0[&spline_key].as_ref().unwrap().clone();
                    let count =
                        u32::from_le_bytes(spline.data[72..76].try_into().unwrap()) as usize;
                    for j in 0..count {
                        put64(&mut spline.data, 112 + j * 1336 + 64, clock.slot);
                    }
                    let bank = execution_bank.with_account(spline_key, spline).unwrap();
                    assert!(
                        bank.execute(ix.clone(), r.input_mint, r.output_mint, r.amount)
                            .is_err()
                    );
                    let mut extended = ix.clone();
                    let n = extended.data.len();
                    extended.data[n - 16..n - 8]
                        .copy_from_slice(&(clock.unix_timestamp + 20).to_le_bytes());
                    if r.input_mint != USDC {
                        assert!(
                            bank.execute(extended.clone(), r.input_mint, r.output_mint, r.amount)
                                .is_err(),
                            "renewed makers alone cannot repair stale withdrawal oracle inputs"
                        );
                    }
                    // Refresh the oracle timestamps too, holding all prices fixed.
                    // This isolates deadline enforcement from both kinds of expiry.
                    let mut prices = cache.0[&ocelot::PERP_MAP].as_ref().unwrap().clone();
                    let n = u32::from_le_bytes(prices.data[32..36].try_into().unwrap()) as usize;
                    for j in 0..n {
                        let o = 64 + j * 1584;
                        if prices.data[o + 1450] != 0 || prices.data[o + 1567] != 0 {
                            continue;
                        }
                        put64(&mut prices.data, o + 16, clock.slot);
                        for start in [32, 168] {
                            for k in 0..5 {
                                put64(&mut prices.data, o + start + k * 16, clock.slot);
                            }
                        }
                        put64(&mut prices.data, o + 264, clock.slot);
                    }
                    let bank = bank.with_account(ocelot::PERP_MAP, prices).unwrap();
                    assert!(
                        bank.execute(ix.clone(), r.input_mint, r.output_mint, r.amount)
                            .is_err()
                    );
                    let result = bank
                        .execute(extended, r.input_mint, r.output_mint, r.amount)
                        .unwrap();
                    assert!(result.output >= *floor);
                    continue;
                }
                if k <= 4 {
                    assert!(exec.is_ok(), "early original transaction must execute");
                }
                if let Ok(e) = exec {
                    assert!(
                        e.output >= *floor,
                        "must never settle below the original output floor"
                    );
                    if k <= 4 {
                        assert!(e.output >= *quoted);
                    }
                    if e.output < *quoted {
                        deteriorated += 1;
                    }
                    successful += 1;
                } else {
                    rejected += 1;
                }
                // Re-preparing on a clock-only change removes expired liquidity.
                if let Ok(q) = refreshed.quote(r.clone()) {
                    let ix = refreshed.build_with_minimum(r, USER, 1).unwrap();
                    assert!(
                        q.expected_output
                            <= execution_bank
                                .execute(ix, r.input_mint, r.output_mint, r.amount)
                                .unwrap()
                                .output
                    );
                }
            }
        }
    }
    assert!(deteriorated > 0 && rejected > 0 && successful > 0);
    eprintln!(
        "original delayed transactions: {successful} settled above floor, {deteriorated} below original estimate, {rejected} rejected before deadline; all expired deadlines rejected"
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn partially_binding_maker_limits_preserve_executable_small_quotes() {
    let (original_venue, original) = fixture(2).await;
    let s = original_venue.state().unwrap();
    let pool = original_venue.pool_id;
    let spline =
        Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &ocelot::PHOENIX).0;
    let idx = &original.0[&s.exchange.indices[0]].as_ref().unwrap().data;
    let buf = &original.0[&s.exchange.buffers[0]].as_ref().unwrap().data;
    let spline_data = &original.0[&spline].as_ref().unwrap().data;
    let count = u32::from_le_bytes(spline_data[72..76].try_into().unwrap()) as usize;
    let mut bases = vec![];
    for j in 0..count {
        let trader = ocelot::codec::key(spline_data, 112 + j * 1336).unwrap();
        let node = idx[96..]
            .chunks_exact(64)
            .position(|d| d[16..48] == trader.to_bytes())
            .unwrap()
            + 1;
        let base = buf[96..]
            .chunks_exact(88)
            .find(|d| {
                u32::from_le_bytes(d[16..20].try_into().unwrap()) as usize == node
                    && u32::from_le_bytes(d[20..24].try_into().unwrap()) as u64 == s.config.asset
            })
            .map(|d| i64::from_le_bytes(d[24..32].try_into().unwrap()))
            .unwrap_or(0);
        bases.push(base);
    }
    let small = request(&original_venue, true, 1_000_000);
    let large = request(&original_venue, true, 1_000_000_000);
    original_venue.quote(large.clone()).unwrap();
    for (mode, maximum) in [
        ("collateral", 1_000_000u32),
        ("haircut", 10_000),
        ("position", 10_000),
    ] {
        let mutate = |value: u32| {
            let mut cache = support::Snapshot(original.0.clone());
            if mode == "collateral" {
                let d = &mut cache
                    .0
                    .get_mut(&s.exchange.indices[0])
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .data;
                for node in d[96..].chunks_exact_mut(64) {
                    let before = i64::from_le_bytes(node[48..56].try_into().unwrap());
                    if before > 0 {
                        put64(
                            node,
                            48,
                            (before as u128 * value as u128 / maximum as u128) as u64,
                        );
                    }
                }
            } else {
                let d = &mut cache.0.get_mut(&spline).unwrap().as_mut().unwrap().data;
                for (j, base) in bases.iter().enumerate() {
                    let o = 112 + j * 1336;
                    if mode == "haircut" {
                        put32(d, o + 1100, 10_000 - value);
                    } else {
                        put32(d, o + 1096, 1);
                        put32(
                            d,
                            o + 1104,
                            u32::try_from((base + value as i64).max(0)).unwrap(),
                        );
                        put32(
                            d,
                            o + 1108,
                            u32::try_from((-base + value as i64).max(0)).unwrap(),
                        );
                    }
                }
            }
            cache
        };
        // Find a fixture mutation with just enough capacity for a small trade.
        // This search sets up the test; production quoting does not search.
        let mut lo = 0;
        let mut hi = maximum;
        assert!(load(&mutate(hi), pool).await.quote(small.clone()).is_ok());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if load(&mutate(mid), pool).await.quote(small.clone()).is_ok() {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        assert!(
            lo > 0 && lo < maximum,
            "{mode}: must bind only part of the depth"
        );
        let cache = mutate(lo);
        let mut v = load(&cache, pool).await;
        let err = v.quote(large.clone()).unwrap_err();
        assert!(format!("{err:?}").contains("spline"), "{mode}: {err:?}");
        v.load_reference_oracle(&cache).await.unwrap();
        let mut accepted = 0;
        for amount in [1_000_000, 2_000_000, 5_000_000, 10_000_000, 100_000_000] {
            let r = request(&v, true, amount);
            if let Ok(q) = v.quote(r.clone()) {
                assert!(
                    q.expected_output <= v.reference_quote(&r).unwrap().output,
                    "{mode} input={amount}"
                );
                accepted += 1;
            }
        }
        assert!(accepted > 0);
        eprintln!(
            "partial maker {mode}: parameter={lo}, accepted comparisons={accepted}, $1000 declined"
        );
    }
}

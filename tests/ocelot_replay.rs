#![cfg(feature = "reference-simulation")]
//! Explicit opt-in replay against captured accounts and actual deployed binaries.
//! No RPC, signing keys or network writes. A missing fixture fails, never skips.
#[allow(dead_code)]
mod support;
use solana_pubkey::Pubkey;
use std::path::Path;
use titan_integration_template::{
    account_caching::AccountsCache,
    ocelot::{self, OcelotVenue, USDC, instruction, simulation::USER},
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};

#[tokio::test]
#[ignore = "build the router SBF binary first; see README"]
async fn deployed_program_replay_and_router_guards() {
    if let Ok(folder) = std::env::var("OCELOT_SNAPSHOT") {
        let market = std::env::var("OCELOT_MARKET").unwrap_or_else(|_| LONG.into());
        replay(&folder, &market).await;
    } else {
        replay("tests/fixtures/sol3l-465928618.bin.gz", LONG).await;
        replay("tests/fixtures/sol2s-465928707.bin.gz", SHORT).await;
        replay("tests/fixtures/sol3l-467430913.bin.gz", LONG).await;
        replay("tests/fixtures/sol2s-467431351.bin.gz", SHORT).await;
    }
}
const LONG: &str = "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM";
const SHORT: &str = "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp";
async fn replay(folder: &str, market: &str) {
    let market: Pubkey = market.parse().unwrap();
    let cache = support::Snapshot::read(Path::new(&folder));
    let mut venue =
        OcelotVenue::from_account(&market, &cache.get_account(&market).await.unwrap().unwrap())
            .unwrap();
    venue.update_state(&cache).await.unwrap();
    venue.load_reference_oracle(&cache).await.unwrap();
    let elf = std::fs::read("program-template/target/deploy/titan_v3_venue_template.so")
        .expect("build router first");
    for mint in [true, false] {
        let supply = venue.state().unwrap().actual_supply;
        let mut amounts = if mint {
            vec![10_000_000, 1_000_000_000]
        } else {
            vec![10_000_000.min(supply), 1_000_000_000.min(supply), supply]
        };
        amounts.sort_unstable();
        amounts.dedup();
        for amount in amounts {
            let (input_mint, output_mint) = if mint {
                (USDC, venue.state().unwrap().market.mint)
            } else {
                (venue.state().unwrap().market.mint, USDC)
            };
            let request = QuoteRequest {
                input_mint,
                output_mint,
                amount,
                swap_type: SwapType::ExactIn,
            };
            let native = venue.quote(request.clone()).unwrap();
            let direct = venue.reference_quote(&request).expect("direct replay");
            assert!(native.expected_output <= direct.output);
            assert!(direct.output > 0);
            assert_eq!(direct.input_remaining, 0);
            for reserve in [None, Some(0), Some(12345)] {
                let (oracle, route) = support::routed(&venue, &request, &elf, reserve);
                let q = oracle
                    .execute(route.clone(), input_mint, output_mint, request.amount)
                    .expect("router replay");
                assert_eq!(
                    q.output, direct.output,
                    "router and direct output must match exactly"
                );
                assert_eq!(q.input_remaining, 0);
                println!(
                    "{folder} mint={mint} input={} output={} route_compute={} packet_bytes={} reserved={reserve:?}",
                    request.amount, q.output, q.compute_units, q.packet_bytes
                );
                let mut changed = route;
                changed.data[1..9].copy_from_slice(&(request.amount + 1).to_le_bytes());
                assert!(
                    oracle
                        .execute(changed, input_mint, output_mint, request.amount + 1)
                        .is_err(),
                    "runtime amount mismatch must fail"
                );
            }
            let direct_ix = venue
                .build_with_minimum(&request, USER, direct.output)
                .unwrap();
            let oracle = venue.execution_oracle().unwrap();
            assert_eq!(
                oracle
                    .execute(direct_ix.clone(), input_mint, output_mint, request.amount)
                    .unwrap()
                    .output,
                direct.output
            );
            for guard in ["min_output", "deadline", "revision"] {
                let mut ix = direct_ix.clone();
                let n = ix.data.len();
                match guard {
                    "min_output" => {
                        ix.data[n - 24..n - 16].copy_from_slice(&(direct.output + 1).to_le_bytes())
                    }
                    "deadline" => ix.data[n - 16..n - 8].copy_from_slice(
                        &(venue.state().unwrap().clock.unix_timestamp - 1).to_le_bytes(),
                    ),
                    _ => ix.data[n - 8..]
                        .copy_from_slice(&(venue.state().unwrap().revision + 1).to_le_bytes()),
                }
                assert!(
                    oracle
                        .execute(ix, input_mint, output_mint, request.amount)
                        .is_err(),
                    "{guard} must fail"
                );
            }
            let mut paused = venue.state().unwrap().accounts[&market].clone();
            paused.data[271] = 1;
            assert!(
                oracle
                    .with_account(market, paused)
                    .unwrap()
                    .execute(direct_ix.clone(), input_mint, output_mint, request.amount)
                    .is_err()
            );
            if mint {
                let mut capped = venue.state().unwrap().accounts[&market].clone();
                capped.data[273..281].copy_from_slice(&1u64.to_le_bytes());
                assert!(
                    oracle
                        .with_account(market, capped)
                        .unwrap()
                        .execute(direct_ix, input_mint, output_mint, request.amount)
                        .is_err()
                );
            }
        }
    }
    // Refresh failure must invalidate the previously loaded execution oracle.
    let mut broken = cache;
    broken
        .0
        .insert(instruction::pda(b"finance_v1", market), None);
    assert!(venue.update_state(&broken).await.is_err());
    assert!(!venue.initialized());
    assert!(venue.execution_oracle().is_err());
    assert_eq!(
        ocelot::PROGRAM_ID.to_string(),
        "CpLah6JAC9967YiVQnQJegfCKEjLEcUYmx3PCYgg9xWf"
    );
}

#[tokio::test]
async fn pinned_execution_outputs_and_non_monotone_lot_boundaries() {
    for (file, key, mint_expected, redeem_expected, mint_boundary, redeem_boundary) in [
        (
            "sol3l-465928618.bin.gz",
            LONG,
            9_263_379,
            10_726_165,
            601_135,
            566_096,
        ),
        (
            "sol2s-465928707.bin.gz",
            SHORT,
            9_752_004,
            10_196_606,
            901_901,
            845_852,
        ),
    ] {
        let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let market: Pubkey = key.parse().unwrap();
        let mut venue =
            OcelotVenue::from_account(&market, &cache.get_account(&market).await.unwrap().unwrap())
                .unwrap();
        venue.update_state(&cache).await.unwrap();
        venue.load_reference_oracle(&cache).await.unwrap();
        for (mint, expected, boundary) in [
            (true, mint_expected, mint_boundary),
            (false, redeem_expected, redeem_boundary),
        ] {
            let (input_mint, output_mint) = if mint {
                (USDC, venue.state().unwrap().market.mint)
            } else {
                (venue.state().unwrap().market.mint, USDC)
            };
            let request = |amount| QuoteRequest {
                input_mint,
                output_mint,
                amount,
                swap_type: SwapType::ExactIn,
            };
            assert_eq!(
                venue.reference_quote(&request(10_000_000)).unwrap().output,
                expected
            );
            let a = venue.reference_quote(&request(boundary - 1)).unwrap();
            let b = venue.reference_quote(&request(boundary)).unwrap();
            assert!(
                a.output > b.output,
                "fixture must reproduce Titan's monotonicity incompatibility"
            );
            assert!(venue.quote(request(10_000_000)).unwrap().expected_output <= expected);
        }
    }
}

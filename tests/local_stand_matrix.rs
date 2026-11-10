//! Local-validator matrix for the sell-off window + surge fee.
//!
//! Reads `scripts/local-stand/stand.json` (built by `scripts/local-stand/up.sh`)
//! and, against the REAL Coffer program running in `solana-test-validator`:
//!
//! - `shared_suite_*` runs Titan's shared suite (`tests/common/mod.rs`) on every
//!   static pool of the configuration matrix and prints a pool × test table;
//! - `real_routed_transactions` sends real `swap_route_v3` transactions through
//!   the Titan router program for every direction of every static pool at
//!   several sizes and compares the user's ATA delta AND the post-swap pool
//!   account with the venue's quote / `apply_swap`;
//! - `dynamic_*` uses the raw contract port to fill entire windows, including
//!   surge regions outside the adapter's supported domain (direct and routed
//!   swaps). It separately checks the adapter's zero-surge boundary and
//!   proves the venue reports a full window as unavailable while the program
//!   reverts with `MaxSelloffExceeded`, waits for a real-time rotation on a
//!   20-second window, toggles `set_token_active` / `set_swaps_enabled` /
//!   `set_pool_enabled`, checks the buy side is unaffected and that the surge
//!   fee accrues in the output token's protocol bucket exactly as predicted.
//!
//! Every test SKIPs when the stand is not up. Run through
//! `scripts/local-stand/run-matrix.sh` (order matters: the suite reads the
//! static pools before the real transactions move them).

#![allow(clippy::result_large_err)]
#![allow(dead_code)] // diagnostic fields kept for the Debug output of failures

mod common;

use std::time::{Duration, Instant};

use solana_pubkey::Pubkey;
use solana_sdk::signature::Signer;

use titan_integration_template::coffer::errors::ErrorCode;
use titan_integration_template::coffer::swap::selloff_headroom;
use titan_integration_template::coffer_venue::CofferVenue;
use titan_integration_template::local_stand::*;
use titan_integration_template::trading_venue::error::TradingVenueError;
use titan_integration_template::trading_venue::{QuoteRequest, SwapType, TradingVenue};

// Allocation guard for the `construction` run under `--profile release-debug`.
#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod stand;
use stand::*;

// ---------------------------------------------------------------------------
// 1. Titan's shared suite on every static pool
// ---------------------------------------------------------------------------

/// The per-direction MVT residual with the input-atom-aware slack, for the
/// table (see README: the shared MVT tolerance is in output atoms only).
fn mvt_residual(venue: &CofferVenue) -> (usize, usize, f64, f64) {
    let mut template_violations = 0;
    let mut input_aware_violations = 0;
    let mut worst: f64 = 0.0;
    let mut worst_after_rounding: f64 = 0.0;
    for (i, j) in venue.directions_num() {
        let Ok((lb, ub)) = venue.bounds(i, j) else {
            continue;
        };
        let (im, om) = (
            venue.get_token(i as usize).unwrap().pubkey,
            venue.get_token(j as usize).unwrap().pubkey,
        );
        let q = |x: u64| {
            venue
                .quote(QuoteRequest {
                    input_mint: im,
                    output_mint: om,
                    amount: x,
                    swap_type: SwapType::ExactIn,
                })
                .unwrap()
        };
        let grid = geometric(lb, ub, 64);
        for w in grid.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b <= a {
                continue;
            }
            let (qa, qb) = (q(a), q(b));
            assert!(
                qb.expected_output >= qa.expected_output,
                "output fell on [{a}, {b}]"
            );
            if qb.expected_output == qa.expected_output {
                continue;
            }
            let chord = (qb.expected_output - qa.expected_output) as f64 / (b - a) as f64;
            let atol = 2.0 / (b - a) as f64;
            if chord > qa.price * (1.0 + 1e-5) + atol || chord < qb.price * (1.0 - 1e-5) - atol {
                template_violations += 1;
                let rel = ((chord / qa.price) - 1.0).max(1.0 - chord / qb.price);
                worst = worst.max(rel);
            }
            let atol2 = (2.0 + qa.price) / (b - a) as f64;
            let over = (chord - (qa.price * (1.0 + 1e-5) + atol2)) / qa.price;
            let under = ((qb.price * (1.0 - 1e-5) - atol2) - chord) / qb.price;
            if over > 0.0 || under > 0.0 {
                input_aware_violations += 1;
                worst_after_rounding = worst_after_rounding.max(over).max(under);
            }
        }
    }
    (
        template_violations,
        input_aware_violations,
        worst,
        worst_after_rounding,
    )
}

async fn shared_suite(only: Option<&str>) {
    let Some(stand) = stand_or_skip("shared_suite").await else {
        return;
    };
    let tests: Vec<&str> = match only {
        Some(t) => vec![t],
        None => SUITE_TESTS.to_vec(),
    };
    type Row = (String, Vec<Result<(), String>>, String);
    let mut table: Vec<Row> = Vec::new();
    for pool in stand.manifest.pools.iter().filter(|p| !p.dynamic) {
        let address: Pubkey = pool.address.parse().unwrap();
        let venue = stand.venue(address).await;
        // The declaration is structural (every ordered pair); a pool with a
        // deactivated input token declares a direction the suite cannot
        // quote, so the suite runs on the quotable subset there and the
        // paused direction is asserted below.
        let dirs = venue.directions_num();
        let quotable: Vec<(u8, u8)> = dirs
            .iter()
            .copied()
            .filter(|&(i, j)| venue.bounds(i, j).is_ok())
            .collect();
        assert_eq!(dirs.len(), pool.tokens.len() * (pool.tokens.len() - 1));
        for &slot in &pool.inactive_tokens {
            assert!(
                dirs.iter().any(|&(i, _)| i == slot),
                "{}: declared",
                pool.case
            );
            assert!(
                quotable.iter().all(|&(i, _)| i != slot),
                "{}: quotable",
                pool.case
            );
        }
        // A disabled pool / disabled swaps declares its directions too, and
        // none of them quotes (`InactivePoolError` at every size, including 0).
        let switched_off = !pool.pool_enabled || !pool.swaps_enabled;
        if switched_off {
            assert!(quotable.is_empty(), "{}: nothing quotable", pool.case);
            for &(i, j) in &dirs {
                for amount in [0, 1, 1_000_000] {
                    assert!(
                        matches!(
                            venue.quote(stand.request(&venue, i, j, amount)),
                            Err(TradingVenueError::InactivePoolError(..))
                        ),
                        "{}: {i}->{j} at {amount}",
                        pool.case
                    );
                }
            }
        }
        let quotable_only = quotable.len() != dirs.len();
        let mut results = Vec::new();
        for t in &tests {
            results.push(run_suite_test(t, address, quotable_only).await);
        }
        let (tv, iv, worst, worst_after_rounding) = mvt_residual(&venue);
        let note = format!(
            "dirs={} quotable={} mvt: template_violations={tv} (worst {worst:.2e}) beyond_input_atom_slack={iv} (worst {worst_after_rounding:.2e})",
            dirs.len(),
            quotable.len()
        );
        table.push((pool.case.clone(), results, note));
    }

    println!(
        "\n== Titan shared suite over the local stand (SOLANA_RPC_URL={}) ==",
        stand.manifest.rpc
    );
    print!("  {:<20}", "case");
    for t in &tests {
        print!(" {:<10}", &t[..t.len().min(10)]);
    }
    println!("  notes");
    let mut failures = Vec::new();
    for (case, results, note) in &table {
        print!("  {case:<20}");
        for (t, r) in tests.iter().zip(results) {
            match r {
                Ok(()) => print!(" {:<10}", "ok"),
                Err(msg) => {
                    print!(" {:<10}", "FAIL");
                    failures.push(format!("{case}/{t}: {}", msg.lines().next().unwrap_or("")));
                }
            }
        }
        println!("  {note}");
    }
    for f in &failures {
        println!("  ! {f}");
    }
    assert!(
        failures.is_empty(),
        "{} suite failure(s) (see table)",
        failures.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_suite_all_tests() {
    shared_suite(None).await;
}

/// `construction` alone, meant for `--profile release-debug` where the
/// no-allocation guard is active.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_suite_construction_no_alloc() {
    shared_suite(Some("construction")).await;
}

// ---------------------------------------------------------------------------
// 2. Real routed transactions through the Titan router
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_routed_transactions() {
    let Some(stand) = stand_or_skip("real_routed_transactions").await else {
        return;
    };
    let mut rows = Vec::new();
    let mut bad = 0;
    for pool in stand.manifest.pools.iter().filter(|p| !p.dynamic) {
        let address: Pubkey = pool.address.parse().unwrap();
        let mut venue = stand.venue(address).await;
        let mut sent = 0;
        let mut mismatches = Vec::new();
        let mut partial_fills = 0;
        let mut surged = 0;
        let mut clock_adjusted = 0;
        let mut max_cu = 0;

        if !pool.pool_enabled || !pool.swaps_enabled {
            // Every direction is unavailable: quoting refuses, the program reverts.
            let q = venue.quote(stand.request(&venue, 0, 1, 1_000_000));
            assert!(
                matches!(q, Err(TradingVenueError::InactivePoolError(..))),
                "{}: {q:?}",
                pool.case
            );
            let ix = venue
                .generate_swap_instruction(
                    stand.request(&venue, 0, 1, 1_000_000),
                    stand.wallet.pubkey(),
                )
                .unwrap();
            let expected = if pool.pool_enabled {
                ErrorCode::SwapsDisabled
            } else {
                ErrorCode::PoolDisabled
            };
            assert_eq!(
                stand.send(&[ix]).await,
                Err(expected.code()),
                "{}",
                pool.case
            );
            rows.push((
                pool.case.clone(),
                format!(
                    "quote refused (InactivePoolError), program reverts {} ({})",
                    expected.name(),
                    expected.code()
                ),
            ));
            continue;
        }

        eprintln!("real_routed_transactions: {} ...", pool.case);
        let mut nothing_fillable = 0;
        let mut exhausted_directions = 0;
        for (i, j) in venue.directions_num() {
            // Earlier samples may have filled this input token's window, and
            // a deactivated input token is declared but unquotable.
            venue = stand.venue(address).await;
            if venue.pool().tokens[i as usize].config.is_active {
                assert_adapter_domain(&venue, i, j);
            }
            let Ok((lb, ub)) = venue.bounds(i, j) else {
                exhausted_directions += 1;
                continue;
            };
            let (in_mint, in_tp) = (
                venue.get_token(i as usize).unwrap().pubkey,
                venue.get_token(i as usize).unwrap().get_token_program(),
            );
            let wallet_has = stand.token_balance(&in_mint, &in_tp).await;
            let ub = ub.min(wallet_has / 2);
            for amount in geometric(lb.max(1), ub.max(lb.max(1)), 3) {
                venue = stand.venue(address).await;
                let p = stand.parity_swap(&venue, i, j, amount, true).await;
                if p.amount_executed == 0 {
                    nothing_fillable += 1;
                    continue;
                }
                sent += 1;
                if p.partial {
                    partial_fills += 1;
                }
                if p.surge_fee > 0 {
                    surged += 1;
                }
                if !p.exact() && p.ok() {
                    clock_adjusted += 1;
                }
                max_cu = max_cu.max(p.compute_units);
                if !p.ok() {
                    mismatches.push(format!("{i}->{j} {p:?}"));
                }
            }
        }
        // Deactivated input token: the direction stays declared but has no
        // quotable range, the quote refuses and the program reverts.
        for &slot in &pool.inactive_tokens {
            assert!(
                venue.directions_num().iter().any(|(i, _)| *i == slot),
                "{}: inactive slot must stay declared",
                pool.case
            );
            assert!(venue.bounds(slot, if slot == 0 { 1 } else { 0 }).is_err());
            let other = if slot == 0 { 1 } else { 0 };
            let q = venue.quote(stand.request(&venue, slot, other, 1_000_000));
            assert!(
                matches!(q, Err(TradingVenueError::AmmMethodError(_))),
                "{}: {q:?}",
                pool.case
            );
            let ix = venue
                .generate_swap_instruction(
                    stand.request(&venue, slot, other, 1_000_000),
                    stand.wallet.pubkey(),
                )
                .unwrap();
            assert_eq!(
                stand.send(&[ix]).await,
                Err(ErrorCode::TokenInactive.code()),
                "{}",
                pool.case
            );
        }
        if !mismatches.is_empty() {
            bad += 1;
        }
        rows.push((
            pool.case.clone(),
            format!(
                "{sent} routed txs, {} exact, {clock_adjusted} exact only at block time, {partial_fills} partial-fill quotes executed at the reported amount, {nothing_fillable} samples / {exhausted_directions} directions with nothing fillable (window exhausted by earlier samples), {surged} with surge fee, max {max_cu} CU{}",
                sent - mismatches.len() - clock_adjusted,
                if mismatches.is_empty() { String::new() } else { format!("; MISMATCH {}", mismatches.join(" | ")) }
            ),
        ));
    }
    println!("\n== real routed transactions (swap_route_v3 on the validator) ==");
    for (case, r) in &rows {
        println!("  {case:<20} {r}");
    }
    assert_eq!(bad, 0, "{bad} pool(s) with on-chain/quote mismatches");
}

// ---------------------------------------------------------------------------
// 3. Dynamic sequences
// ---------------------------------------------------------------------------

/// Sell BONK in chunks until the window is full; each chunk must match
/// on-chain (payout and full pool state), the full window must be reported
/// as unavailable and reverted on-chain, and the buy side must keep working.
/// Returns (sold, per-chunk parity, output-token protocol bucket right after the sells).
async fn fill_window(
    stand: &Stand,
    case: &str,
    routed: bool,
    chunks: u64,
) -> (u64, Vec<Parity>, u64) {
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let mut venue = stand.venue(pool).await;
    let probe = contract_headroom_of(&venue, 0, 1);
    assert!(
        probe.not_enough_liquidity,
        "{case}: probe should exceed the cap"
    );
    let cap = probe.amount;
    assert!(cap > 0);
    let chunk = cap / chunks;
    let mut log = Vec::new();
    let mut sold = 0u64;
    loop {
        venue = stand.venue(pool).await;
        let head = contract_headroom_of(&venue, 0, 1).amount;
        if head == 0 {
            break;
        }
        let amount = chunk.min(head);
        assert_adapter_domain(&venue, 0, 1);
        let p = stand
            .contract_parity_swap(&venue, 0, 1, amount, routed)
            .await;
        assert_eq!(
            p.amount_executed, amount,
            "raw contract swap executes the requested size"
        );
        assert!(p.ok(), "{case}: chunk {amount} mismatch: {p:?}");
        if !p.exact() {
            println!(
                "  {case}: chunk {amount} exact only at block time ({:?})",
                p.clock_adjusted.unwrap()
            );
        }
        sold += p.amount_executed;
        log.push(p);
    }
    // The raw contract path fills the full window, including zero-output
    // atoms at a 100% surge rate; adapter support is checked separately.
    venue = stand.venue(pool).await;
    let bucket_after_sells = venue.pool().tokens[1].dynamics.protocol_fees_owed;
    let raw_head = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap_or(0);
    assert!(
        venue.directions_num().contains(&(0, 1)) && venue.directions_num().contains(&(1, 0)),
        "{case}: both directions stay declared (the window rotates on its own)"
    );
    assert!(
        venue.bounds(0, 1).is_err(),
        "{case}: bounds must report no quotable range"
    );
    let q = venue.quote(stand.request(&venue, 0, 1, 1)).unwrap();
    assert!(
        q.not_enough_liquidity && q.amount == 0 && q.expected_output == 0,
        "{case}: {q:?}"
    );
    // One atom past the cap reverts on-chain.
    let ix = venue
        .generate_swap_instruction(
            stand.request(&venue, 0, 1, raw_head + 1),
            stand.wallet.pubkey(),
        )
        .unwrap();
    assert_eq!(
        stand.send(&[ix]).await,
        Err(ErrorCode::MaxSelloffExceeded.code()),
        "{case}"
    );
    assert_eq!(raw_head, 0, "{case}: the raw contract window must be full");
    // Buy side unaffected by BONK's cap.
    let (lb, ub) = venue.bounds(1, 0).unwrap();
    let p = stand
        .parity_swap(&venue, 1, 0, geometric(lb, ub, 3)[1], routed)
        .await;
    assert!(p.ok(), "{case}: buy side mismatch {p:?}");
    (sold, log, bucket_after_sells)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dynamic_window_filling_direct_swaps() {
    let Some(stand) = stand_or_skip("dynamic_window_filling_direct_swaps").await else {
        return;
    };
    println!("\n== dynamic: window filling with direct swaps ==");
    for case in ["dyn_b", "dyn_c", "dyn_8020_c"] {
        let pool: Pubkey = stand.pool(case).address.parse().unwrap();
        let before = stand.pool_state(pool).await;
        let (sold, log, bucket_after_sells) = fill_window(&stand, case, false, 7).await;
        let after = stand.pool_state(pool).await;
        let surge_total: u64 = log.iter().map(|p| p.surge_fee).sum();
        let accrued = bucket_after_sells - before.tokens[1].dynamics.protocol_fees_owed;
        assert!(
            surge_total > 0,
            "{case}: full contract sequence must cover surge fees"
        );
        // The LP swap-fee protocol cut lands on the INPUT token; the surge fee
        // is the only thing that lands on the OUTPUT token's bucket.
        assert_eq!(
            accrued, surge_total,
            "{case}: surge accrual in USDC protocol bucket"
        );
        assert_eq!(
            after.tokens[0].dynamics.current_selloff, sold,
            "{case}: window accumulator"
        );
        println!(
            "  {case:<12} {} chunks, sold {sold} (cap {}), surge fee accrued {accrued} USDC atoms across {} surged chunks, window full -> unavailable + MaxSelloffExceeded, buy side ok",
            log.len(),
            after.tokens[0].dynamics.selloff_vb_snapshot
                * stand.pool(case).selloff[0].max_selloff_pct as u64
                / 10_000,
            log.iter().filter(|p| p.surge_fee > 0).count()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dynamic_window_filling_routed() {
    let Some(stand) = stand_or_skip("dynamic_window_filling_routed").await else {
        return;
    };
    println!("\n== dynamic: window filling through the Titan router (config b, kink at 50%) ==");
    let (sold, log, _) = fill_window(&stand, "dyn_router", true, 10).await;
    assert!(log.iter().any(|p| p.surge_fee > 0));
    for (k, p) in log.iter().enumerate() {
        println!(
            "  chunk {k}: in {} out {} surge {}",
            p.amount_executed, p.received, p.surge_fee
        );
    }
    println!("  sold {sold}; {} routed chunks all exact", log.len());
}

/// 100% surge above the threshold: the venue admits the last untaxed
/// quantized fill cell; a subsequent raw contract swap pays nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dynamic_full_fee_exhaustion_point() {
    let Some(stand) = stand_or_skip("dynamic_full_fee_exhaustion_point").await else {
        return;
    };
    let case = "dyn_d";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let venue = stand.venue(pool).await;
    let cap = venue.pool().tokens[0].dynamics.virtual_balance / 10;
    let threshold_point = ((cap as u128 * 8_001 - 1) / 10_000) as u64;
    let q = venue.quote(stand.request(&venue, 0, 1, cap)).unwrap();
    assert!(q.not_enough_liquidity, "{q:?}");
    assert_eq!(
        q.amount, threshold_point,
        "exhaustion point is the end of the untaxed 80% fill cell"
    );
    assert!(q.price > 0.0);
    let (_, ub) = venue.bounds(0, 1).unwrap();
    assert!(
        ub <= threshold_point && threshold_point - ub <= 100,
        "ub {ub} vs {threshold_point}"
    );
    // Execute at the exhaustion point: exact parity.
    let p = stand
        .parity_swap(&venue, 0, 1, threshold_point, false)
        .await;
    assert!(p.ok(), "{p:?}");
    // Now sell MORE than the quote admits (10% of the cap past the threshold)
    // for real: the program accepts it and charges all of its output as surge.
    let venue = stand.venue(pool).await;
    let more = cap / 10;
    let q = venue.quote(stand.request(&venue, 0, 1, more)).unwrap();
    assert!(
        q.not_enough_liquidity && q.amount == 0 && q.expected_output == 0,
        "{q:?}"
    );
    let raw = stand.contract_parity_swap(&venue, 0, 1, more, false).await;
    assert!(raw.ok(), "full-fee contract parity: {raw:?}");
    let received = raw.received;
    println!(
        "\n== dynamic: 100% surge (dyn_d) == exhaustion point {threshold_point} executed exactly; selling {more} more on-chain paid {received} atoms (quote: 0 fillable, 0 output)"
    );
    assert_eq!(
        received, 0,
        "above the exhaustion point the program pays nothing"
    );
}

/// 20-second window: fill it, watch the real clock rotate it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dynamic_real_time_rotation() {
    let Some(stand) = stand_or_skip("dynamic_real_time_rotation").await else {
        return;
    };
    let case = "dyn_short";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let period = stand.pool(case).selloff[0].period_length as i64;
    // Start from a fresh window: wait until two periods have passed since the
    // window opened (pool creation), so the state is clean.
    let mut venue = stand.venue(pool).await;
    let opened = venue.pool().tokens[0].dynamics.window_start_timestamp;
    let now = venue.now();
    if now < opened + 2 * period + 1 {
        tokio::time::sleep(Duration::from_secs((opened + 2 * period + 1 - now) as u64)).await;
    }
    let t0 = Instant::now();
    let (sold, log, _) = fill_window(&stand, case, false, 4).await;
    println!("\n== dynamic: real-time rotation (dyn_short, period {period} s) ==");
    println!(
        "  filled the window: sold {sold} in {} chunks ({:.1} s)",
        log.len(),
        t0.elapsed().as_secs_f64()
    );
    venue = stand.venue(pool).await;
    let d = venue.pool().tokens[0].dynamics;
    let window_start = d.window_start_timestamp;
    // Wait for one rotation.
    let wake = window_start + period + 2;
    let now = stand.venue(pool).await.now();
    if wake > now {
        tokio::time::sleep(Duration::from_secs((wake - now) as u64)).await;
    }
    venue = stand.venue(pool).await;
    // The contract has new capacity after one rotation even if the decaying
    // carry-over still places it inside the surge zone. Adapter availability
    // follows its stricter zero-surge boundary.
    assert_adapter_domain(&venue, 0, 1);
    let head = contract_headroom_of(&venue, 0, 1).amount;
    assert!(
        head > 0 && head < sold,
        "after one rotation the carry-over leaves partial headroom: {head} vs sold {sold}"
    );
    let p = stand.contract_parity_swap(&venue, 0, 1, head, false).await;
    assert!(p.ok(), "sell exactly the post-rotation headroom: {p:?}");
    venue = stand.venue(pool).await;
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, 1), stand.wallet.pubkey())
        .unwrap();
    let r = stand.send(&[ix]).await;
    // The weighted carry-over decays every second, so one more atom may already
    // fit by the time the tx lands; both outcomes are consistent with the quote
    // at the moment it was made.
    println!(
        "  after one period: headroom {head} sold exactly (parity ok); +1 atom -> {:?} (clock moved {} s since the quote)",
        r,
        stand.venue(pool).await.now() - venue.now()
    );
    // The post-rotation sell rotated the window once more (it ran at
    // elapsed >= period): count the "two periods" from the NEW window start.
    let window_start = stand.venue(pool).await.pool().tokens[0]
        .dynamics
        .window_start_timestamp;
    let wake = window_start + 2 * period + 2;
    let now = stand.venue(pool).await.now();
    if wake > now {
        tokio::time::sleep(Duration::from_secs((wake - now) as u64)).await;
    }
    venue = stand.venue(pool).await;
    let head2 = contract_headroom_of(&venue, 0, 1).amount;
    let cap_now = venue.pool().tokens[0].dynamics.virtual_balance / 10;
    println!("  after two periods: headroom {head2} == cap {cap_now} (fresh window)");
    assert_eq!(head2, cap_now);
    assert_adapter_domain(&venue, 0, 1);
    assert!(
        venue.bounds(0, 1).is_ok(),
        "zero-surge domain recovers in a fresh window"
    );
    let p = stand.parity_swap(&venue, 0, 1, cap_now / 3, false).await;
    assert!(p.ok(), "{p:?}");
}

/// Deactivate / reactivate BONK, disable / enable swaps and the pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dynamic_toggles() {
    let Some(stand) = stand_or_skip("dynamic_toggles").await else {
        return;
    };
    let case = "dyn_toggle";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let me = stand.wallet.pubkey();
    println!("\n== dynamic: toggles (dyn_toggle) ==");

    // deactivate BONK
    stand
        .send(&[set_token_active_ix(LOCAL_CONFIG, pool, me, 0, false)])
        .await
        .unwrap();
    let venue = stand.venue(pool).await;
    assert_eq!(
        venue.directions_num(),
        vec![(0, 1), (1, 0)],
        "declaration is structural"
    );
    assert!(
        venue.bounds(0, 1).is_err(),
        "no quotable range while deactivated"
    );
    assert!(matches!(
        venue.quote(stand.request(&venue, 0, 1, 1_000_000)),
        Err(TradingVenueError::AmmMethodError(_))
    ));
    assert!(venue.quote(stand.request(&venue, 0, 1, 0)).is_err());
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, 1_000_000), me)
        .unwrap();
    assert_eq!(
        stand.send(&[ix]).await,
        Err(ErrorCode::TokenInactive.code())
    );
    let (lb, ub) = venue.bounds(1, 0).unwrap();
    let p = stand
        .parity_swap(&venue, 1, 0, geometric(lb, ub, 3)[1], true)
        .await;
    assert!(p.ok(), "buying the deactivated token via the router: {p:?}");
    println!(
        "  BONK deactivated: direction still declared, no quotable range, quote refuses (TokenInactive), program reverts 6054, buying BONK via router exact"
    );
    // reactivate
    stand
        .send(&[set_token_active_ix(LOCAL_CONFIG, pool, me, 0, true)])
        .await
        .unwrap();
    let venue = stand.venue(pool).await;
    assert!(venue.directions_num().contains(&(0, 1)));
    let (lb, ub) = venue.bounds(0, 1).expect("quotable again once reactivated");
    let p = stand
        .parity_swap(&venue, 0, 1, geometric(lb, ub, 3)[1], true)
        .await;
    assert!(p.ok(), "{p:?}");
    println!("  BONK reactivated: direction back, routed sell exact");

    // swaps disabled
    stand
        .send(&[set_swaps_enabled_ix(LOCAL_CONFIG, pool, me, false)])
        .await
        .unwrap();
    let venue = stand.venue(pool).await;
    assert!(matches!(
        venue.quote(stand.request(&venue, 0, 1, 1_000)),
        Err(TradingVenueError::InactivePoolError(..))
    ));
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, 1_000), me)
        .unwrap();
    assert_eq!(
        stand.send(&[ix]).await,
        Err(ErrorCode::SwapsDisabled.code())
    );
    stand
        .send(&[set_swaps_enabled_ix(LOCAL_CONFIG, pool, me, true)])
        .await
        .unwrap();
    println!(
        "  swaps disabled: InactivePoolError, program reverts SwapsDisabled (6021); re-enabled"
    );

    // pool disabled (protocol admin)
    stand
        .send(&[set_pool_enabled_ix(LOCAL_CONFIG, pool, me, false)])
        .await
        .unwrap();
    let venue = stand.venue(pool).await;
    assert!(matches!(
        venue.quote(stand.request(&venue, 0, 1, 1_000)),
        Err(TradingVenueError::InactivePoolError(..))
    ));
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, 1_000), me)
        .unwrap();
    assert_eq!(stand.send(&[ix]).await, Err(ErrorCode::PoolDisabled.code()));
    stand
        .send(&[set_pool_enabled_ix(LOCAL_CONFIG, pool, me, true)])
        .await
        .unwrap();
    let venue = stand.venue(pool).await;
    let p = stand.parity_swap(&venue, 0, 1, 1_000_000, true).await;
    assert!(p.ok(), "{p:?}");
    println!(
        "  pool disabled: InactivePoolError, program reverts PoolDisabled (6020); re-enabled, routed swap exact"
    );
}

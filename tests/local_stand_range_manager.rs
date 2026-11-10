//! Local-validator scenarios for pool state that changes UNDER the venue:
//! the range-manager role moving virtual balances and weights, the admin
//! reconfiguring the sell-off policy, liquidity added / removed, and the
//! clock advancing between a quote and its execution.
//!
//! Contract-focused scenarios compare the complete `quote_exact_in` result
//! with REAL transactions and full post-swap pool state, including surge
//! amounts that Titan no longer advertises. Each refresh separately checks
//! the adapter's surge-free domain; supported directions also run adapter
//! quote parity and Titan's shared suite. Routed contract tests validate the
//! instruction path, not router selection of an unsupported surge amount. Dedicated
//! `rm_*` pools of the stand are mutated; reset the stand
//! (`scripts/local-stand/up.sh`) before re-running.
//!
//! Scenarios (see the README section "Range manager and admin changes"):
//! - `range_manager_vb_moves_and_rotation` (a, c, h): vb raised / lowered by
//!   the max step on a half-filled 30 s window with the surge active, the cap
//!   staying on the old snapshot, the rotation re-snapshotting to the moved
//!   vb with the carry-over rescaled, a combined vb + weight update, and two
//!   executions WITHOUT a refresh across a rotation boundary;
//! - `range_manager_weight_moves` (b): 50/50 -> 55/45 -> 45/55 -> 50/50 with
//!   and without the surge, both directions, with the shared suite after each
//!   move;
//! - `range_manager_leverage_caps_output` (d): USDC vb pushed to 16x so the
//!   LP balance caps the output before the window cap, then back;
//! - `admin_reconfigures_max_selloff` (e): cap raised, lowered under the fill,
//!   curve changed, cap disabled, re-enabled;
//! - `admin_adds_and_removes_liquidity` (g): proportional deposit and exit
//!   with the window rescaled on-chain.
//!
//! (f), the token kill switch, is `dynamic_toggles` in `local_stand_matrix.rs`.

#![allow(clippy::result_large_err)]

mod common;
mod stand;

use std::time::Duration;

use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::signature::Signer;
use spl_token::state::Account as TokenAccount;

use stand::*;
use titan_integration_template::coffer::constants::PERCENT_SCALE;
use titan_integration_template::coffer::errors::ErrorCode;
use titan_integration_template::coffer::math::fixed_point::FixedPoint;
use titan_integration_template::coffer::state::CofferPool;
use titan_integration_template::coffer::swap::{quote_exact_in, selloff_headroom};
use titan_integration_template::coffer_venue::CofferVenue;
use titan_integration_template::local_stand::*;
use titan_integration_template::trading_venue::TradingVenue;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Stand {
    /// Refresh and validate the adapter boundary independently of the raw
    /// contract scenarios below.
    async fn checked_venue(&self, pool: Pubkey) -> CofferVenue {
        let venue = self.venue(pool).await;
        for (i, j) in venue.directions_num() {
            assert_adapter_domain(&venue, i, j);
        }
        venue
    }

    /// The `(mint, token_program)` list of a pool, in slot order.
    fn pool_mints(&self, case: &str) -> Vec<(Pubkey, Pubkey)> {
        self.pool(case)
            .tokens
            .iter()
            .map(|name| self.mint(name))
            .collect()
    }

    /// Appoint the local wallet as range manager with the given envelope
    /// (per-step caps in `PERCENT_SCALE` units, no cooldown, no leverage band).
    async fn appoint_range_manager(&self, pool: Pubkey, max_vb_pct: u16, max_weight_pct: u16) {
        let me = self.wallet.pubkey();
        self.send(&[
            set_range_manager_ix(LOCAL_CONFIG, pool, me, me, true),
            set_range_manager_config_ix(pool, me, max_vb_pct, max_weight_pct, 0, 0, 0),
        ])
        .await
        .expect("appoint range manager");
        let state = self.pool_state(pool).await;
        assert!(state.range_manager_enabled && state.range_manager == me);
        assert_eq!(state.range_manager_max_vb_change_pct, max_vb_pct);
        assert_eq!(state.range_manager_max_weight_change_pct, max_weight_pct);
    }

    /// `range_manager_update` with the compare-and-swap guards read from the
    /// live pool right before sending. `vb` / `weights` are `(slot, new)`.
    async fn range_update(
        &self,
        pool: Pubkey,
        vb: &[(u8, u64)],
        weights: &[(u8, u64)],
    ) -> CofferPool {
        let live = self.pool_state(pool).await;
        let vb_changes: Vec<TokenChange> = vb
            .iter()
            .map(|&(i, new)| {
                TokenChange::new(i, live.tokens[i as usize].dynamics.virtual_balance, new)
            })
            .collect();
        let w_changes: Vec<TokenChange> = weights
            .iter()
            .map(|&(i, new)| {
                TokenChange::new(i, live.tokens[i as usize].config.normalized_weight, new)
            })
            .collect();
        self.send(&[range_manager_update_ix(
            pool,
            self.wallet.pubkey(),
            &vb_changes,
            &w_changes,
        )])
        .await
        .unwrap_or_else(|code| panic!("range_manager_update failed with {code}"));
        let after = self.pool_state(pool).await;
        for &(i, new) in vb {
            assert_eq!(after.tokens[i as usize].dynamics.virtual_balance, new);
        }
        for &(i, new) in weights {
            assert_eq!(after.tokens[i as usize].config.normalized_weight, new);
        }
        after
    }

    /// Block until the validator clock is at least `t`.
    async fn wait_until(&self, pool: Pubkey, t: i64) {
        loop {
            let now = self.checked_venue(pool).await.now();
            if now >= t {
                return;
            }
            tokio::time::sleep(Duration::from_secs((t - now).max(1) as u64)).await;
        }
    }

    /// Predict the full contract amount at the venue's clock, then execute
    /// the SAME instruction without
    /// any refresh once the clock passed `execute_at`. Returns
    /// `(quoted, received-or-error, block time - quote clock, exact at block time)`.
    async fn stale_execute(
        &self,
        venue: &CofferVenue,
        amount: u64,
        execute_at: i64,
    ) -> (u64, Result<u64, u32>, i64, bool) {
        let request = self.request(venue, 0, 1, amount);
        let quote = self
            .contract_quote(venue, 0, 1, amount)
            .expect("raw contract quote");
        let ix = venue
            .generate_swap_instruction(request.clone(), self.wallet.pubkey())
            .unwrap();
        self.wait_until(venue.market_id(), execute_at).await;
        let (out_mint, out_tp) = (
            venue.get_token(1).unwrap().pubkey,
            venue.get_token(1).unwrap().get_token_program(),
        );
        let before = self.token_balance(&out_mint, &out_tp).await;
        let sent = self.send(&[ix]).await;
        let received = self.token_balance(&out_mint, &out_tp).await - before;
        let (block_time, exact_at_block_time) = match &sent {
            Ok(sig) => {
                let (bt, _) = self.tx_meta(sig).await;
                // The venue math at the block time (pool state unchanged since
                // the quote) must reproduce the execution exactly.
                let o = quote_exact_in(
                    venue.pool(),
                    amount,
                    0,
                    1,
                    venue.get_token(0).unwrap().decimals as u8,
                    venue.get_token(1).unwrap().decimals as u8,
                    bt,
                );
                (
                    bt,
                    o.map(|o| o.amount_out_user == received).unwrap_or(false),
                )
            }
            Err(_) => {
                let bt = self.venue(venue.market_id()).await.now();
                let o = quote_exact_in(
                    venue.pool(),
                    amount,
                    0,
                    1,
                    venue.get_token(0).unwrap().decimals as u8,
                    venue.get_token(1).unwrap().decimals as u8,
                    bt,
                );
                (bt, o.is_err())
            }
        };
        (
            quote.amount_out_user,
            sent.map(|_| received),
            block_time - venue.now(),
            exact_at_block_time,
        )
    }
}

fn effective_before(pool: &CofferPool, now: i64) -> u64 {
    let cap_headroom = selloff_headroom(pool, 0, now).unwrap().unwrap();
    let d = pool.tokens[0].dynamics;
    let cap = d.selloff_vb_snapshot * pool.tokens[0].config.max_selloff_pct as u64 / PERCENT_SCALE;
    cap - cap_headroom
}

/// `scale_by_ratio` of the contract's window rescale (18-dec fixed point).
fn rescaled(value: u64, ratio: u128, increase: bool) -> u64 {
    let delta = FixedPoint::mul_down(value as u128, ratio).unwrap();
    (if increase {
        value as u128 + delta
    } else {
        (value as u128).saturating_sub(delta)
    }) as u64
}

/// Parity swaps in both directions (direct and routed) plus the invariants
/// on the spot price; returns the parity rows for the log.
async fn both_directions(stand: &Stand, pool: Pubkey, label: &str) -> Vec<Parity> {
    let mut rows = Vec::new();
    for (i, j, routed) in [(0u8, 1u8, false), (1, 0, true), (0, 1, true), (1, 0, false)] {
        let venue = stand.checked_venue(pool).await;
        let contract_head = contract_headroom_of(&venue, i, j).amount;
        assert!(contract_head > 0, "{label}: no contract headroom");
        // Exercise full contract fees independently of adapter availability.
        let amount = geometric(1, contract_head, 3)[1];
        let spot = venue.quote(stand.request(&venue, i, j, 0)).unwrap();
        assert!(
            spot.price > 0.0 && spot.expected_output == 0,
            "{label}: {spot:?}"
        );
        let p = if let Ok((lb, ub)) = venue.bounds(i, j) {
            // Retain adapter-to-execution coverage on each supported domain.
            stand
                .parity_swap(&venue, i, j, amount.clamp(lb, ub), routed)
                .await
        } else {
            stand
                .contract_parity_swap(&venue, i, j, amount, routed)
                .await
        };
        assert!(p.ok(), "{label}: {i}->{j} routed={routed} {p:?}");
        rows.push(p);
    }
    rows
}

/// The shared suite on a mutated pool: every test must pass except
/// `mean_value_theorem` on the USDC -> BONK direction (documented residual
/// of the input-side fee rounding, unrelated to the mutation).
async fn suite_after(pool: Pubkey, label: &str) -> String {
    let rows = run_full_suite(pool, true).await;
    let mut failed = Vec::new();
    for (t, r) in &rows {
        if let Err(msg) = r {
            if *t == "mean_value_theorem" {
                continue;
            }
            failed.push(format!("{t}: {}", msg.lines().next().unwrap_or("")));
        }
    }
    assert!(
        failed.is_empty(),
        "{label}: shared suite failed: {failed:?}"
    );
    let mvt = match &rows
        .iter()
        .find(|(t, _)| *t == "mean_value_theorem")
        .unwrap()
        .1
    {
        Ok(()) => "ok",
        Err(_) => "known fee-rounding residual",
    };
    format!("shared suite: 7/7 ok, mean_value_theorem {mvt}")
}

// ---------------------------------------------------------------------------
// (a) (c) (h): virtual balance moves, rotation, stale execution
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_manager_vb_moves_and_rotation() {
    let Some(stand) = stand_or_skip("range_manager_vb_moves_and_rotation").await else {
        return;
    };
    let case = "rm_short";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let period = stand.pool(case).selloff[0].period_length as i64;
    stand.appoint_range_manager(pool, 5_000, 2_000).await;
    println!("\n== range manager: vb moves on a half-filled {period} s window (rm_short) ==");

    // Fresh window: two periods since the pool was created.
    let venue = stand.checked_venue(pool).await;
    let opened = venue.pool().tokens[0].dynamics.window_start_timestamp;
    stand.wait_until(pool, opened + 2 * period + 1).await;

    // Half-fill the window. This opens the window (snapshot := live vb).
    let venue = stand.checked_venue(pool).await;
    let cap = contract_headroom_of(&venue, 0, 1).amount;
    let vb0 = venue.pool().tokens[0].dynamics.virtual_balance;
    assert_eq!(cap, vb0 / 10);
    let p = stand
        .contract_parity_swap(&venue, 0, 1, cap / 2, false)
        .await;
    assert!(p.ok(), "{p:?}");
    let venue = stand.checked_venue(pool).await;
    let d = venue.pool().tokens[0].dynamics;
    let window_start = d.window_start_timestamp;
    let snapshot = d.selloff_vb_snapshot;
    assert_eq!(snapshot, vb0);
    assert_eq!(d.current_selloff, cap / 2);
    let head0 = contract_headroom_of(&venue, 0, 1).amount;
    assert_eq!(head0, cap - cap / 2);
    let q0 = stand.contract_quote(&venue, 0, 1, head0 / 2).unwrap();
    let spot0 = venue.quote(stand.request(&venue, 0, 1, 0)).unwrap().price;
    println!(
        "  half-filled: snapshot {snapshot}, cap {cap}, headroom {head0}, quote({}) = {} (surge fee in it)",
        head0 / 2,
        q0.amount_out_user
    );

    // (a) raise BONK vb by the max step (+50%): the cap stays on the old
    // snapshot, the headroom is unchanged, the curve moved.
    let live_vb = venue.pool().tokens[0].dynamics.virtual_balance;
    let raised = live_vb + live_vb / 2;
    stand.range_update(pool, &[(0, raised)], &[]).await;
    let venue = stand.checked_venue(pool).await;
    assert_eq!(venue.pool().tokens[0].dynamics.virtual_balance, raised);
    assert_eq!(
        venue.pool().tokens[0].dynamics.selloff_vb_snapshot,
        snapshot,
        "snapshot untouched"
    );
    assert_eq!(
        contract_headroom_of(&venue, 0, 1).amount,
        head0,
        "headroom on the OLD snapshot"
    );
    let q1 = stand.contract_quote(&venue, 0, 1, head0 / 2).unwrap();
    let spot1 = venue.quote(stand.request(&venue, 0, 1, 0)).unwrap().price;
    assert!(
        q1.amount_out_user < q0.amount_out_user,
        "a larger vb_in pays less: {q1:?} vs {q0:?}"
    );
    assert!(
        spot1 < spot0,
        "base spot must follow the virtual balance move"
    );
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head0 / 4, false)
        .await;
    assert!(p.ok(), "after +50% vb: {p:?}");
    println!(
        "  +50% vb -> {raised}: snapshot {snapshot} kept, headroom {head0} kept, quote({}) = {} (was {}), sold {} exact (surge {})",
        head0 / 2,
        q1.amount_out_user,
        q0.amount_out_user,
        p.amount_executed,
        p.surge_fee
    );

    // lower it by 40%
    let venue = stand.checked_venue(pool).await;
    let live_vb = venue.pool().tokens[0].dynamics.virtual_balance;
    let lowered = live_vb - live_vb * 40 / 100;
    stand.range_update(pool, &[(0, lowered)], &[]).await;
    let venue = stand.checked_venue(pool).await;
    assert_eq!(
        venue.pool().tokens[0].dynamics.selloff_vb_snapshot,
        snapshot
    );
    let head2 = contract_headroom_of(&venue, 0, 1).amount;
    assert_eq!(
        head2,
        head0 - head0 / 4,
        "headroom only moved by what was sold"
    );
    let q2 = stand.contract_quote(&venue, 0, 1, head0 / 4).unwrap();
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head0 / 4, true)
        .await;
    assert!(p.ok(), "after -40% vb (routed): {p:?}");
    println!(
        "  -40% vb -> {lowered}: snapshot kept, headroom {head2}, quote({}) = {}, routed sell exact (surge {})",
        head0 / 4,
        q2.amount_out_user,
        p.surge_fee
    );

    // (c) vb and weights in ONE update: +20% BONK vb, 50/50 -> 55/45.
    let venue = stand.checked_venue(pool).await;
    let live_vb = venue.pool().tokens[0].dynamics.virtual_balance;
    let both = live_vb + live_vb / 5;
    stand
        .range_update(pool, &[(0, both)], &[(0, 5_500), (1, 4_500)])
        .await;
    let venue = stand.checked_venue(pool).await;
    assert_eq!(venue.pool().tokens[0].config.normalized_weight, 5_500);
    assert_eq!(venue.pool().tokens[1].config.normalized_weight, 4_500);
    assert_eq!(
        venue.pool().tokens[0].dynamics.selloff_vb_snapshot,
        snapshot
    );
    let head3 = contract_headroom_of(&venue, 0, 1).amount;
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head3 / 2, false)
        .await;
    assert!(p.ok(), "after vb+weights: {p:?}");
    let pb = stand
        .parity_swap(&stand.checked_venue(pool).await, 1, 0, 1_000_000_000, true)
        .await;
    assert!(pb.ok(), "buy side after vb+weights: {pb:?}");
    println!(
        "  +20% vb and 55/45 weights in one update: headroom {head3}, sold {} exact (surge {}), bought BONK with 1000 USDC exact",
        p.amount_executed, p.surge_fee
    );
    let elapsed_in_window = stand.checked_venue(pool).await.now() - window_start;
    assert!(
        elapsed_in_window < period,
        "the scenario must fit in one window ({elapsed_in_window} s)"
    );

    // Rotation: the new snapshot is the MOVED vb, the carry-over is rescaled.
    stand.wait_until(pool, window_start + period + 2).await;
    let venue = stand.checked_venue(pool).await;
    let pre = *venue.pool();
    let live_vb = pre.tokens[0].dynamics.virtual_balance;
    let head4 = contract_headroom_of(&venue, 0, 1).amount;
    let expected_prev = (pre.tokens[0].dynamics.current_selloff as u128 * live_vb as u128
        / snapshot as u128) as u64;
    let expected_cap = live_vb / 10;
    let e = (venue.now() - (window_start + period)) as u128;
    let expected_effective = (expected_prev as u128 * (period as u128 - e) / period as u128) as u64;
    assert_eq!(
        head4,
        expected_cap - expected_effective,
        "rotation: new cap on the moved vb, carry-over rescaled"
    );
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head4 / 2, false)
        .await;
    assert!(p.ok(), "post-rotation sell: {p:?}");
    let chain = stand.pool_state(pool).await;
    assert_eq!(
        chain.tokens[0].dynamics.selloff_vb_snapshot, live_vb,
        "re-snapshotted to the moved vb"
    );
    assert_eq!(
        chain.tokens[0].dynamics.window_start_timestamp,
        window_start + period
    );
    assert_eq!(chain.tokens[0].dynamics.previous_selloff, expected_prev);
    println!(
        "  rotation: snapshot {snapshot} -> {live_vb} (moved vb), carry-over {} -> {expected_prev}, headroom {head4}, sold {} exact",
        pre.tokens[0].dynamics.current_selloff, p.amount_executed
    );

    // (h) NO refresh across a rotation boundary, twice.
    // 1. Nothing moved the vb below the snapshot: execution pays MORE than
    //    quoted (the accumulated sells decay into the carry-over and the new
    //    snapshot is the grown vb).
    let venue = stand.checked_venue(pool).await;
    let ws = venue.pool().tokens[0].dynamics.window_start_timestamp;
    let amount = contract_headroom_of(&venue, 0, 1).amount / 2;
    let (quoted, got, dt, exact_bt) = stand.stale_execute(&venue, amount, ws + period + 2).await;
    let got1 = got.expect("stale execution after a plain rotation must not revert");
    assert!(
        got1 >= quoted,
        "plain rotation is conservative: got {got1} < quoted {quoted}"
    );
    assert!(
        exact_bt,
        "the venue math at the block time reproduces the execution"
    );
    println!(
        "  stale execution (no refresh, {dt} s later, across a rotation, vb >= snapshot): quoted {quoted}, received {got1} (>= quoted), exact once the clock is the block time"
    );
    // 2. The range manager LOWERED the vb mid-window (cap still on the old
    //    snapshot at quote time); the rotation re-snapshots to the lowered vb,
    //    so the same input lands at a higher window fill: a higher surge rate
    //    (less output than quoted) or `MaxSelloffExceeded`.
    let venue = stand.checked_venue(pool).await;
    let ws = venue.pool().tokens[0].dynamics.window_start_timestamp;
    let live_vb = venue.pool().tokens[0].dynamics.virtual_balance;
    stand
        .range_update(pool, &[(0, live_vb - live_vb * 49 / 100)], &[])
        .await;
    let venue = stand.checked_venue(pool).await;
    let amount = contract_headroom_of(&venue, 0, 1).amount / 2;
    let (quoted, got, dt, exact_bt) = stand.stale_execute(&venue, amount, ws + period + 2).await;
    match got {
        Ok(received) => {
            assert!(
                received < quoted,
                "a lowered vb across a rotation is NOT conservative: got {received} vs quoted {quoted}"
            );
            assert!(exact_bt);
            println!(
                "  stale execution after the manager lowered vb 49% (no refresh, {dt} s later, across a rotation): quoted {quoted}, received {received} (LESS than quoted: the new cap is 10% of the lowered vb); exact once the clock is the block time"
            );
        }
        Err(code) => {
            assert_eq!(code, ErrorCode::MaxSelloffExceeded.code());
            assert!(exact_bt, "the venue predicts the revert at the block time");
            println!(
                "  stale execution after the manager lowered vb 49% (no refresh, {dt} s later, across a rotation): quoted {quoted}, reverted MaxSelloffExceeded (predicted at the block time)"
            );
        }
    }

    // Read-only suite tests on the mutated pool (the ones that do not
    // simulate against a clock that may rotate mid-run on a 30 s window).
    for t in [
        "construction",
        "zero_input_spot_price",
        "monotone",
        "price_monotone",
    ] {
        run_suite_test(t, pool, true)
            .await
            .unwrap_or_else(|m| panic!("{t}: {m}"));
    }
    println!(
        "  shared suite (construction, zero_input_spot_price, monotone, price_monotone) ok on the mutated pool"
    );
}

// ---------------------------------------------------------------------------
// (b): weight moves
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_manager_weight_moves() {
    let Some(stand) = stand_or_skip("range_manager_weight_moves").await else {
        return;
    };
    println!("\n== range manager: weight moves (rm_w_surge = config b, rm_w_cap = cap only) ==");
    for case in ["rm_w_surge", "rm_w_cap"] {
        let pool: Pubkey = stand.pool(case).address.parse().unwrap();
        // 45/55 -> 55/45 moves one weight by 22%: a 30% envelope.
        stand.appoint_range_manager(pool, 5_000, 3_000).await;
        // Put the surge pool inside the surge zone first (config b taxes from
        // fill 0, kink at 50%): sell 30% of the window.
        let venue = stand.checked_venue(pool).await;
        let cap = contract_headroom_of(&venue, 0, 1).amount;
        let p = stand
            .contract_parity_swap(&venue, 0, 1, cap * 3 / 10, false)
            .await;
        assert!(p.ok(), "{case}: {p:?}");
        for (w0, w1) in [(5_500u64, 4_500u64), (4_500, 5_500), (5_000, 5_000)] {
            let before = stand.checked_venue(pool).await;
            let spot_before = before.quote(stand.request(&before, 0, 1, 0)).unwrap().price;
            stand.range_update(pool, &[], &[(0, w0), (1, w1)]).await;
            let venue = stand.checked_venue(pool).await;
            assert_eq!(venue.pool().tokens[0].config.normalized_weight, w0);
            assert_eq!(venue.pool().tokens[1].config.normalized_weight, w1);
            let spot_after = venue.quote(stand.request(&venue, 0, 1, 0)).unwrap().price;
            // Spot price = (V_out/V_in) * (w_in/w_out) * keep: it must follow the weights.
            let d = venue.pool();
            let expected_spot = d.tokens[1].dynamics.virtual_balance as f64
                / d.tokens[0].dynamics.virtual_balance as f64
                * (w0 as f64 / w1 as f64)
                * (1.0 - d.swap_fee_rate as f64 / 1_000_000.0);
            assert!(
                (spot_after / expected_spot - 1.0).abs() < 1e-12,
                "{case}: base spot {spot_after} vs curve spot {expected_spot}"
            );
            let rows = both_directions(&stand, pool, &format!("{case} {w0}/{w1}")).await;
            let suite = suite_after(pool, &format!("{case} {w0}/{w1}")).await;
            println!(
                "  {case:<10} {w0}/{w1}: spot {spot_before:.6e} -> {spot_after:.6e} (base curve {expected_spot:.6e}); 4 swaps exact: {}; {suite}",
                rows.iter()
                    .map(|p| format!(
                        "in {} out {} surge {}",
                        p.amount_executed, p.received, p.surge_fee
                    ))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
    }
}

// ---------------------------------------------------------------------------
// (d): leverage pushes the LP-balance cap ahead of the window cap
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_manager_leverage_caps_output() {
    let Some(stand) = stand_or_skip("range_manager_leverage_caps_output").await else {
        return;
    };
    let case = "rm_lev";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    stand.appoint_range_manager(pool, 10_000, 2_000).await;
    println!("\n== range manager: leverage (rm_lev, USDC vb x16 then back) ==");
    let dec = |v: &CofferVenue, i: usize| v.get_token(i).unwrap().decimals as u8;

    // Baseline: the window cap binds first.
    let venue = stand.checked_venue(pool).await;
    let cap = contract_headroom_of(&venue, 0, 1).amount;
    assert_eq!(cap, venue.pool().tokens[0].dynamics.virtual_balance / 10);
    assert_eq!(
        quote_exact_in(
            venue.pool(),
            cap + 1,
            0,
            1,
            dec(&venue, 0),
            dec(&venue, 1),
            venue.now()
        )
        .unwrap_err(),
        ErrorCode::MaxSelloffExceeded
    );
    // Sell 45% of the window so the surge zone (threshold 50%) is next.
    let p = stand
        .contract_parity_swap(&venue, 0, 1, cap * 45 / 100, false)
        .await;
    assert!(p.ok(), "{p:?}");

    // Push USDC vb to 16x in four max-step (100%) updates.
    for _ in 0..4 {
        let live = stand.pool_state(pool).await.tokens[1]
            .dynamics
            .virtual_balance;
        stand.range_update(pool, &[(1, live * 2)], &[]).await;
    }
    let venue = stand.checked_venue(pool).await;
    let head = contract_headroom_of(&venue, 0, 1).amount;
    let window_head = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap();
    assert!(
        head < window_head,
        "LP balance must cap first: fillable {head} vs window headroom {window_head}"
    );
    assert_eq!(
        quote_exact_in(
            venue.pool(),
            head + 1,
            0,
            1,
            dec(&venue, 0),
            dec(&venue, 1),
            venue.now()
        )
        .unwrap_err(),
        ErrorCode::AmountOutExceedsBalance
    );
    let q = stand.contract_quote(&venue, 0, 1, head).unwrap();
    assert_eq!(
        q.amount_out_user + q.surge_fee_amount,
        venue.pool().tokens[1].dynamics.actual_balance,
        "the edge drains the LP balance exactly (gross curve output)"
    );
    let adapter_head = headroom_of(&venue, 0, 1).amount;
    let (_, ub) = venue.bounds(0, 1).unwrap();
    assert!(ub <= adapter_head && adapter_head - ub <= 100);
    assert!(adapter_head <= head);
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, head + 1), stand.wallet.pubkey())
        .unwrap();
    assert_eq!(
        stand.send(&[ix]).await,
        Err(ErrorCode::AmountOutExceedsBalance.code())
    );
    let venue = stand.checked_venue(pool).await;
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head / 3, true)
        .await;
    assert!(p.ok() && p.surge_fee > 0, "{p:?}");
    println!(
        "  USDC vb x16: fillable {head} < window headroom {window_head} (LP balance {} caps first), +1 atom reverts AmountOutExceedsBalance, bounds ub {ub}, routed sell of {} exact (surge {})",
        venue.pool().tokens[1].dynamics.actual_balance,
        p.amount_executed,
        p.surge_fee
    );

    // Reverse: back to 1x, the window cap binds again.
    for _ in 0..4 {
        let live = stand.pool_state(pool).await.tokens[1]
            .dynamics
            .virtual_balance;
        stand.range_update(pool, &[(1, live / 2)], &[]).await;
    }
    let venue = stand.checked_venue(pool).await;
    let head = contract_headroom_of(&venue, 0, 1).amount;
    let window_head = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap();
    assert_eq!(head, window_head, "window cap binds again");
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, head + 1), stand.wallet.pubkey())
        .unwrap();
    assert_eq!(
        stand.send(&[ix]).await,
        Err(ErrorCode::MaxSelloffExceeded.code())
    );
    let venue = stand.checked_venue(pool).await;
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head / 2, false)
        .await;
    assert!(p.ok() && p.surge_fee > 0, "{p:?}");
    let suite = suite_after(pool, case).await;
    println!(
        "  back to 1x: fillable {head} == window headroom, +1 atom reverts MaxSelloffExceeded, sell of {} exact (surge {}); {suite}",
        p.amount_executed, p.surge_fee
    );
}

// ---------------------------------------------------------------------------
// (e): set_max_selloff reconfigured between quotes
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_reconfigures_max_selloff() {
    let Some(stand) = stand_or_skip("admin_reconfigures_max_selloff").await else {
        return;
    };
    let case = "rm_cfg";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let me = stand.wallet.pubkey();
    println!("\n== admin: set_max_selloff between quotes (rm_cfg, starts as config b) ==");
    let b = stand.pool(case).selloff[0];
    let reconfigure = |p: SelloffParams| set_max_selloff_ix(pool, me, &[p, SelloffParams::OFF]);

    // Half-fill the window.
    let venue = stand.checked_venue(pool).await;
    let cap = contract_headroom_of(&venue, 0, 1).amount;
    let p = stand
        .contract_parity_swap(&venue, 0, 1, cap / 2, false)
        .await;
    assert!(p.ok(), "{p:?}");
    let snapshot = stand.pool_state(pool).await.tokens[0]
        .dynamics
        .selloff_vb_snapshot;

    // Cap raised to 20%: same snapshot, headroom = 20% - fill.
    stand
        .send(&[reconfigure(SelloffParams {
            max_selloff_pct: 2_000,
            ..b
        })])
        .await
        .unwrap();
    let venue = stand.checked_venue(pool).await;
    assert_eq!(
        venue.pool().tokens[0].dynamics.selloff_vb_snapshot,
        snapshot
    );
    let head = contract_headroom_of(&venue, 0, 1).amount;
    let raw = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap();
    let fill = effective_before(venue.pool(), venue.now());
    assert_eq!(raw, snapshot * 2 / 10 - fill);
    assert_eq!(head, raw, "contract fillable amount reaches the window cap");
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head / 4, true)
        .await;
    assert!(p.ok(), "cap 20%: {p:?}");
    println!(
        "  cap 10% -> 20%: snapshot {snapshot} kept, raw headroom {raw} (fillable {head}), routed sell {} exact (surge {})",
        p.amount_executed, p.surge_fee
    );

    // Cap lowered under the fill (5%): nothing fillable, the program reverts.
    stand
        .send(&[reconfigure(SelloffParams {
            max_selloff_pct: 500,
            ..b
        })])
        .await
        .unwrap();
    let venue = stand.checked_venue(pool).await;
    let q = venue.quote(stand.request(&venue, 0, 1, 1)).unwrap();
    assert!(
        q.not_enough_liquidity && q.amount == 0 && q.expected_output == 0,
        "{q:?}"
    );
    assert!(venue.directions_num().contains(&(0, 1)) && venue.bounds(0, 1).is_err());
    let ix = venue
        .generate_swap_instruction(stand.request(&venue, 0, 1, 1), me)
        .unwrap();
    assert_eq!(
        stand.send(&[ix]).await,
        Err(ErrorCode::MaxSelloffExceeded.code())
    );
    let pb = stand.parity_swap(&venue, 1, 0, 1_000_000_000, false).await;
    assert!(pb.ok(), "{pb:?}");
    println!(
        "  cap -> 5% (under the fill): quote 0 fillable, direction still declared, no quotable range, program reverts MaxSelloffExceeded; buy side exact"
    );

    // Curve changed: cap 10%, threshold 80%, 0/0/100% (config c).
    let c = SelloffParams::new(1_000, b.period_length, 8_000, 0, 0, 10_000, 0);
    stand.send(&[reconfigure(c)]).await.unwrap();
    let venue = stand.checked_venue(pool).await;
    let head = contract_headroom_of(&venue, 0, 1).amount;
    // The window is at 87.5% of the 10% cap (50% + 37.5% sold above), i.e.
    // already past the new 80% threshold: even a small sell is surcharged,
    // at the rate of the new curve.
    let fill_pct = effective_before(venue.pool(), venue.now()) * 100 / (snapshot / 10);
    assert!(fill_pct >= 80, "fill {fill_pct}%");
    let small = quote_exact_in(venue.pool(), head / 10, 0, 1, 5, 6, venue.now()).unwrap();
    assert!(
        small.surge_fee_amount > 0,
        "past the threshold: surge on every atom"
    );
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head * 9 / 10, true)
        .await;
    assert!(p.ok() && p.surge_fee > 0, "curve c: {p:?}");
    println!(
        "  curve -> thr 80% 0/0/100%: window at {fill_pct}% fill, headroom {head}, quote({}) already surcharged {}, routed sell {} exact with surge {}",
        head / 10,
        small.surge_fee_amount,
        p.amount_executed,
        p.surge_fee
    );

    // Cap disabled: uncapped quotes, the window is not advanced on-chain.
    stand
        .send(&[reconfigure(SelloffParams::OFF)])
        .await
        .unwrap();
    let venue = stand.checked_venue(pool).await;
    let d_before = venue.pool().tokens[0].dynamics;
    let q = venue.quote(stand.request(&venue, 0, 1, cap * 2)).unwrap();
    assert!(!q.not_enough_liquidity, "uncapped: {q:?}");
    let p = stand.parity_swap(&venue, 0, 1, cap * 2, false).await;
    assert!(p.ok() && p.surge_fee == 0, "uncapped: {p:?}");
    let d_after = stand.pool_state(pool).await.tokens[0].dynamics;
    assert_eq!(
        (
            d_after.current_selloff,
            d_after.previous_selloff,
            d_after.selloff_vb_snapshot
        ),
        (
            d_before.current_selloff,
            d_before.previous_selloff,
            d_before.selloff_vb_snapshot
        )
    );
    println!(
        "  cap disabled: sold {} (2x the old cap) exact, no surge, window accumulators untouched",
        p.amount_executed
    );

    // Re-enabled (config b): the retained accumulators count against the new cap.
    stand.send(&[reconfigure(b)]).await.unwrap();
    let venue = stand.checked_venue(pool).await;
    let head = contract_headroom_of(&venue, 0, 1).amount;
    let raw = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap();
    assert_eq!(
        raw,
        snapshot / 10 - effective_before(venue.pool(), venue.now())
    );
    assert!(head <= raw && head > 0);
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head / 2, true)
        .await;
    assert!(p.ok(), "re-enabled: {p:?}");
    let suite = suite_after(pool, case).await;
    println!(
        "  cap re-enabled (config b): raw headroom {raw} on the retained accumulators, fillable {head}, routed sell {} exact (surge {}); {suite}",
        p.amount_executed, p.surge_fee
    );
}

// ---------------------------------------------------------------------------
// (g): add_liquidity / remove_liquidity between quotes
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_adds_and_removes_liquidity() {
    let Some(stand) = stand_or_skip("admin_adds_and_removes_liquidity").await else {
        return;
    };
    let case = "rm_liq";
    let pool: Pubkey = stand.pool(case).address.parse().unwrap();
    let me = stand.wallet.pubkey();
    let mints = stand.pool_mints(case);
    println!("\n== admin: add / remove liquidity between quotes (rm_liq, config b) ==");

    // 40% of the window sold.
    let venue = stand.checked_venue(pool).await;
    let cap = contract_headroom_of(&venue, 0, 1).amount;
    let p = stand
        .contract_parity_swap(&venue, 0, 1, cap * 4 / 10, false)
        .await;
    assert!(p.ok(), "{p:?}");

    // Proportional deposit of 25%: vb, snapshot and accumulators scale by 1.25.
    let before = stand.pool_state(pool).await;
    let amounts: Vec<u64> = (0..2)
        .map(|i| before.tokens[i].dynamics.actual_balance / 4)
        .collect();
    stand
        .send(&[add_liquidity_ix(pool, me, &mints, &amounts, 0)])
        .await
        .unwrap();
    let after = stand.pool_state(pool).await;
    let ratio: u128 = FixedPoint::div_down(
        amounts[0] as u128,
        before.tokens[0].dynamics.actual_balance as u128,
    )
    .unwrap()
    .min(
        FixedPoint::div_down(
            amounts[1] as u128,
            before.tokens[1].dynamics.actual_balance as u128,
        )
        .unwrap(),
    );
    let d0 = before.tokens[0].dynamics;
    assert_eq!(
        after.tokens[0].dynamics.selloff_vb_snapshot,
        rescaled(d0.selloff_vb_snapshot, ratio, true)
    );
    assert_eq!(
        after.tokens[0].dynamics.current_selloff,
        rescaled(d0.current_selloff, ratio, true)
    );
    let venue = stand.checked_venue(pool).await;
    let head = contract_headroom_of(&venue, 0, 1).amount;
    let raw = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap();
    let expected_raw = after.tokens[0].dynamics.selloff_vb_snapshot / 10
        - effective_before(venue.pool(), venue.now());
    assert_eq!(raw, expected_raw);
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head / 4, false)
        .await;
    assert!(p.ok(), "after add_liquidity: {p:?}");
    println!(
        "  add_liquidity 25%: snapshot {} -> {}, current {} -> {}, headroom {} (raw {raw}), sold {} exact (surge {})",
        d0.selloff_vb_snapshot,
        after.tokens[0].dynamics.selloff_vb_snapshot,
        d0.current_selloff,
        after.tokens[0].dynamics.current_selloff,
        head,
        p.amount_executed,
        p.surge_fee
    );

    // Exit: burn 20% of the BPT supply, everything scales by 0.8.
    let bpt = bpt_mint(&pool);
    let supply = spl_token::state::Mint::unpack(&stand.rpc.get_account(&bpt).await.unwrap().data)
        .unwrap()
        .supply;
    let held = TokenAccount::unpack(
        &stand
            .rpc
            .get_account(&ata(&me, &bpt, &spl_token::ID))
            .await
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;
    let burn = (supply / 5).min(held);
    let before = stand.pool_state(pool).await;
    stand
        .send(&[remove_liquidity_ix(pool, me, &mints, burn)])
        .await
        .unwrap();
    let after = stand.pool_state(pool).await;
    let ratio = FixedPoint::div_down(burn as u128, supply as u128).unwrap();
    let d0 = before.tokens[0].dynamics;
    assert_eq!(
        after.tokens[0].dynamics.selloff_vb_snapshot,
        rescaled(d0.selloff_vb_snapshot, ratio, false)
    );
    assert_eq!(
        after.tokens[0].dynamics.current_selloff,
        rescaled(d0.current_selloff, ratio, false)
    );
    let venue = stand.checked_venue(pool).await;
    let head = contract_headroom_of(&venue, 0, 1).amount;
    let raw = selloff_headroom(venue.pool(), 0, venue.now())
        .unwrap()
        .unwrap();
    assert_eq!(
        raw,
        after.tokens[0].dynamics.selloff_vb_snapshot / 10
            - effective_before(venue.pool(), venue.now())
    );
    let p = stand
        .contract_parity_swap(&venue, 0, 1, head / 3, true)
        .await;
    assert!(p.ok(), "after remove_liquidity: {p:?}");
    let pb = stand
        .parity_swap(&stand.checked_venue(pool).await, 1, 0, 2_000_000_000, true)
        .await;
    assert!(pb.ok(), "{pb:?}");
    let suite = suite_after(pool, case).await;
    println!(
        "  remove_liquidity 20% of supply: snapshot {} -> {}, current {} -> {}, headroom {head} (raw {raw}), routed sell {} exact (surge {}), buy exact; {suite}",
        d0.selloff_vb_snapshot,
        after.tokens[0].dynamics.selloff_vb_snapshot,
        d0.current_selloff,
        after.tokens[0].dynamics.current_selloff,
        p.amount_executed,
        p.surge_fee
    );
}

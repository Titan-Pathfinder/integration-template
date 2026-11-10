// PORTED VERBATIM from the Coffer contract's `src/math/max_selloff.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! Sliding-window cumulative-sell rate limiter.
//!
//! For each token, the pool may cap how much of that token can be **sold
//! INTO** the pool over a configured window. The cap is a **percentage of
//! the token's `virtual_balance`** (`max_selloff_pct`, in `PERCENT_SCALE`
//! units where 10_000 = 100%), resolved against a **per-window snapshot**
//! of that virtual balance — NOT the live value.
//!
//! ## Why a snapshot (security)
//! If the cap were `pct * live_virtual_balance`, an attacker could inflate
//! the base inside the same window (selling raises the input token's vb;
//! a proportional add-liquidity raises it too) so the percentage resolves
//! to a larger absolute number — the classic "circuit breaker's dilemma"
//! / base-inflation bypass (Euler, Cream). We instead capture
//! `selloff_vb_snapshot` when a window opens and hold it for the window,
//! so the absolute cap is fixed within a window and only re-baselines when
//! a new window starts (auto-scaling across windows as the pool grows).
//!
//! ## Sliding window (Cloudflare / Solend RateLimiter style)
//! ```text
//!     cap       = floor(max_selloff_pct * vb_snapshot / PERCENT_SCALE)
//!     effective = previous * (period - elapsed) / period + current
//!     require: effective + amount_in <= cap
//! ```
//! Bucket rotation:
//!   - `elapsed >= 2*period`  → both buckets aged out; clear, `window_start
//!     = now`, re-snapshot vb.
//!   - `elapsed >= period`    → one boundary crossed; `previous = current`,
//!     `current = 0`, `window_start += period`, re-snapshot vb.
//!   - `elapsed < period`     → no rotation; reuse the stored snapshot (or
//!     capture it if still uninitialised).
//!
//! All arithmetic uses `u128` intermediates so the `u64` stores never
//! overflow even at `vb = u64::MAX`. The cap is floored (protocol-favoring:
//! a user can never sell slightly more than the configured fraction).

use crate::coffer::prelude::*;

use crate::coffer::constants::PERCENT_SCALE;
use crate::coffer::errors::ErrorCode;
use crate::coffer::math::fixed_point::FixedPoint;
use crate::coffer::state::{AssetDynamics, CofferPool};

/// Apply the same proportional move to a stored window figure that the
/// token's `virtual_balance` just underwent.
///
/// `ratio` is 18-decimal fixed point; `is_increase` picks `v * (1 + ratio)`
/// or `v * (1 - ratio)`, mirroring exactly how `add_liquidity` and
/// `remove_liquidity` move `virtual_balance`.
fn scale_by_ratio(value: u64, ratio: u128, is_increase: bool) -> Result<u64> {
    if value == 0 {
        return Ok(0);
    }
    let v = value as u128;
    let delta = FixedPoint::mul_down(v, ratio)?;
    let out = if is_increase {
        v.checked_add(delta).ok_or(ErrorCode::MathOverflow)?
    } else {
        // Rounding can only ever make `delta` smaller than the true share,
        // so this cannot go negative; saturating keeps it total anyway.
        v.saturating_sub(delta)
    };
    out.try_into().map_err(|_| ErrorCode::MathOverflow.into())
}

/// Keep the sell-off window in step with a liquidity-driven move of
/// `virtual_balance`.
///
/// The window cap is `max_selloff_pct * selloff_vb_snapshot`, and the
/// snapshot is taken once when a window opens — deliberately, so selling
/// inside a window cannot inflate the basis it is measured against. But
/// `add_liquidity` and `remove_liquidity` also move `virtual_balance`, and
/// they used to leave the window untouched. That opened the cap to a
/// capital-free bypass: deposit to inflate the balance, open the window with
/// a dust sell so the snapshot locks in the inflated value, withdraw the
/// deposit in full, and the cap stays inflated for the rest of the window
/// with none of the capital still at risk. The surge fee, priced off
/// `sold / cap`, reads the oversized sell as small and collects nothing.
///
/// The mirror case is just as real: shrinking `virtual_balance` right as a
/// window rotates can leave `previous_selloff` alone above the freshly-taken
/// cap, so every honest sell reverts until the carryover decays away.
///
/// Scaling the snapshot **and** both accumulators by the same ratio keeps
/// the window's fill fraction — `sold / cap` — invariant across liquidity
/// events, which is the property both attacks rely on breaking. Tokens with
/// the cap disabled are skipped: their window state is meaningless until an
/// admin opts in, and `check_and_advance` re-captures the snapshot then.
pub fn rescale_window_on_liquidity_change(
    pool: &mut CofferPool,
    token_count: usize,
    ratio: u128,
    is_increase: bool,
) -> Result<()> {
    if ratio == 0 {
        return Ok(());
    }
    for i in 0..token_count {
        if pool.tokens[i].config.max_selloff_pct == 0 {
            continue;
        }
        let d = &mut pool.tokens[i].dynamics;
        d.selloff_vb_snapshot = scale_by_ratio(d.selloff_vb_snapshot, ratio, is_increase)?;
        d.previous_selloff = scale_by_ratio(d.previous_selloff, ratio, is_increase)?;
        d.current_selloff = scale_by_ratio(d.current_selloff, ratio, is_increase)?;
    }
    Ok(())
}

/// Per-call summary of what the check observed. Used to emit
/// `MaxSelloffWindowAdvanced` once per swap.
#[derive(Debug, Clone, Copy)]
pub struct MaxSelloffResult {
    /// `(previous * (period - elapsed) / period) + current` — the window's
    /// fill BEFORE this swap's `amount_in` is added. The surge fee integrates
    /// its rate from here to `effective_selloff`, which is what makes the fee
    /// path-independent (splitting a sell cannot cheapen it).
    pub effective_selloff_before: u64,
    /// `(previous * (period - elapsed) / period) + current + amount_in`,
    /// i.e. the value compared against the resolved cap.
    pub effective_selloff: u64,
    /// Resolved absolute cap for this window =
    /// `floor(max_selloff_pct * vb_snapshot / PERCENT_SCALE)`.
    pub max_selloff_cap: u64,
    /// Virtual-balance snapshot the cap was resolved against.
    pub vb_snapshot: u64,
    /// Post-update `previous_selloff` (may have rolled from `current`).
    pub previous_selloff: u64,
    /// Post-update `current_selloff` (always includes `amount_in`).
    pub current_selloff: u64,
    /// Post-update `window_start_timestamp` (may have advanced).
    pub window_start_timestamp: i64,
}

/// Run the sliding-window check for one token and mutate its dynamics
/// in-place. Idempotent on rejection — if the function returns
/// `Err(MaxSelloffExceeded)`, no field on `dynamics` is changed.
///
/// Caller is responsible for the gate `max_selloff_pct > 0`; passing `0`
/// here short-circuits as "disabled" (returns `Ok(None)`).
///
/// # Parameters
/// * `dynamics`         — token's mutable `AssetDynamics` row.
/// * `max_selloff_pct`  — cap as a percent of the vb snapshot, in
///   `PERCENT_SCALE` units (10_000 = 100%). Must be `<= PERCENT_SCALE`.
/// * `period`           — sliding-window length in seconds. Must be `> 0`
///   when `max_selloff_pct > 0`; otherwise `MaxSelloffInvalidConfig`.
/// * `amount_in`        — swap's `amount_in` (gross, pre-fee) to add to the
///   running total.
/// * `virtual_balance`  — the input token's CURRENT virtual balance
///   (pre-swap). Captured as the snapshot when a window opens.
/// * `now`              — `Clock::unix_timestamp`.
///
/// # Errors
/// `MaxSelloffInvalidConfig`, `MaxSelloffExceeded`, `MathOverflow`.
pub fn check_and_advance(
    dynamics: &mut AssetDynamics,
    max_selloff_pct: u64,
    period: u32,
    amount_in: u64,
    virtual_balance: u64,
    now: i64,
) -> Result<Option<MaxSelloffResult>> {
    if max_selloff_pct == 0 {
        return Ok(None);
    }
    require!(period > 0, ErrorCode::MaxSelloffInvalidConfig);
    // Defence-in-depth: also enforced on the config write path.
    require!(
        max_selloff_pct <= PERCENT_SCALE,
        ErrorCode::MaxSelloffInvalidConfig
    );

    let period_i: i64 = period as i64;

    // Stake-weighted median `unix_timestamp` isn't strictly monotonic.
    // Clamp negative elapsed (clock skew) to 0 rather than rotating
    // backwards.
    let raw_elapsed = now.saturating_sub(dynamics.window_start_timestamp);
    let mut elapsed = if raw_elapsed < 0 { 0 } else { raw_elapsed };

    // Roll forward whole periods. Compute the new state into locals first
    // so we can bail on rejection without partial mutation. `opened_window`
    // marks whether a fresh window started → re-snapshot the vb basis.
    let two_periods = period_i
        .checked_mul(2)
        .ok_or(ErrorCode::MathOverflow)?;
    let (new_previous, new_current, new_window_start, elapsed_in_window, opened_window) =
        if elapsed >= two_periods {
            (0u64, 0u64, now, 0i64, true)
        } else if elapsed >= period_i {
            // `current` becomes `previous`, current resets; window slides
            // by exactly one period (not jumping to `now` — keeps the
            // formula's interpolation stable).
            let new_ws = dynamics
                .window_start_timestamp
                .checked_add(period_i)
                .ok_or(ErrorCode::MathOverflow)?;
            elapsed = now.saturating_sub(new_ws);
            if elapsed < 0 { elapsed = 0; }
            (dynamics.current_selloff, 0u64, new_ws, elapsed, true)
        } else {
            (
                dynamics.previous_selloff,
                dynamics.current_selloff,
                dynamics.window_start_timestamp,
                elapsed,
                false,
            )
        };

    // Snapshot the vb basis: capture the current vb when a window opens, or
    // when the snapshot is still uninitialised (first check after enabling).
    // Otherwise reuse the stored snapshot so the cap can't be inflated by
    // selling / depositing into the token mid-window.
    let vb_snapshot = if opened_window || dynamics.selloff_vb_snapshot == 0 {
        virtual_balance
    } else {
        dynamics.selloff_vb_snapshot
    };

    // The carryover was accumulated against the PREVIOUS window's snapshot.
    // When a rotation re-baselines the snapshot, the carryover has to move
    // into the new scale with it — otherwise a `virtual_balance` that shrank
    // across the boundary (an LP exit, or an ordinary buy) can leave
    // `previous_selloff` alone above the freshly-taken cap, and every sell of
    // that token reverts with `MaxSelloffExceeded` despite zero real sell
    // pressure since the rotation.
    let old_snapshot = dynamics.selloff_vb_snapshot;
    let new_previous = if opened_window && old_snapshot > 0 && vb_snapshot != old_snapshot {
        let rescaled = (new_previous as u128)
            .checked_mul(vb_snapshot as u128)
            .ok_or(ErrorCode::MathOverflow)?
            / (old_snapshot as u128);
        rescaled.try_into().map_err(|_| ErrorCode::MathOverflow)?
    } else {
        new_previous
    };

    // Resolve the absolute cap, floored (protocol-favoring). Since
    // `max_selloff_pct <= PERCENT_SCALE`, `cap <= vb_snapshot` and fits u64.
    let cap_u128 = (max_selloff_pct as u128)
        .checked_mul(vb_snapshot as u128)
        .ok_or(ErrorCode::MathOverflow)?
        / (PERCENT_SCALE as u128);
    let max_selloff_cap: u64 = cap_u128
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?;

    // effective = previous * (period - elapsed) / period + current + amount_in
    let period_u = period_i as u128;
    // Defence-in-depth: by branch invariants elapsed_in_window ∈ [0, period),
    // but `checked_sub` ensures any future refactor can't silently underflow.
    let remaining = period_u
        .checked_sub(elapsed_in_window as u128)
        .ok_or(ErrorCode::MathOverflow)?;
    let weighted_prev = (new_previous as u128)
        .checked_mul(remaining)
        .ok_or(ErrorCode::MathOverflow)?
        / period_u;
    let pre_effective = weighted_prev
        .checked_add(new_current as u128)
        .ok_or(ErrorCode::MathOverflow)?;
    let effective_total = pre_effective
        .checked_add(amount_in as u128)
        .ok_or(ErrorCode::MathOverflow)?;

    require!(
        effective_total <= max_selloff_cap as u128,
        ErrorCode::MaxSelloffExceeded
    );

    // Commit. `current += amount_in` is bounded by `cap` (≤ u64), so the
    // cast is safe.
    let new_current_with_amount = (new_current as u128)
        .checked_add(amount_in as u128)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(
        new_current_with_amount <= u64::MAX as u128,
        ErrorCode::MathOverflow
    );

    dynamics.previous_selloff = new_previous;
    dynamics.current_selloff = new_current_with_amount as u64;
    dynamics.window_start_timestamp = new_window_start;
    dynamics.selloff_vb_snapshot = vb_snapshot;

    Ok(Some(MaxSelloffResult {
        effective_selloff_before: pre_effective as u64,
        effective_selloff: effective_total as u64,
        max_selloff_cap,
        vb_snapshot,
        previous_selloff: new_previous,
        current_selloff: dynamics.current_selloff,
        window_start_timestamp: new_window_start,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(window_start: i64) -> AssetDynamics {
        AssetDynamics {
            virtual_balance: 0,
            actual_balance: 0,
            protocol_fees_owed: 0,
            previous_selloff: 0,
            current_selloff: 0,
            window_start_timestamp: window_start,
            selloff_vb_snapshot: 0,
        }
    }

    // vb = 10_000 with pct = 10_000 (100%) gives an absolute cap of 10_000,
    // matching the old absolute-cap tests so the window mechanics stay
    // directly comparable.
    const VB: u64 = 10_000;
    const FULL: u64 = PERCENT_SCALE; // 100% → cap == VB

    #[test]
    fn disabled_returns_none() {
        let mut d = fresh(0);
        let r = check_and_advance(&mut d, 0, 0, 1_000, VB, 0).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn within_first_window_accumulates() {
        let mut d = fresh(0);
        let r = check_and_advance(&mut d, FULL, 60, 300, VB, 10).unwrap().unwrap();
        assert_eq!(r.effective_selloff, 300);
        assert_eq!(r.max_selloff_cap, VB);
        assert_eq!(r.vb_snapshot, VB);
        assert_eq!(d.current_selloff, 300);
        assert_eq!(d.previous_selloff, 0);
        assert_eq!(d.selloff_vb_snapshot, VB);
    }

    #[test]
    fn cap_is_percent_of_snapshot() {
        // 10% of vb=10_000 → cap=1_000. Selling 1_000 is OK, 1_001 fails.
        let mut d = fresh(0);
        let pct = 1_000; // 10%
        let r = check_and_advance(&mut d, pct, 60, 1_000, VB, 0).unwrap().unwrap();
        assert_eq!(r.max_selloff_cap, 1_000);
        // next 1 unit in same window must exceed the 1_000 cap.
        let err = check_and_advance(&mut d, pct, 60, 1, VB, 1).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffExceeded"));
    }

    #[test]
    fn cap_floors_down() {
        // pct = 1 (0.01%), vb = 9_999 → 1*9_999/10_000 = 0 (floored).
        let mut d = fresh(0);
        let err = check_and_advance(&mut d, 1, 60, 1, 9_999, 0).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffExceeded"));
        assert_eq!(d.current_selloff, 0, "state must not mutate on reject");
    }

    #[test]
    fn snapshot_not_inflated_by_growing_vb_within_window() {
        // First swap snapshots vb=10_000 → cap=1_000 (10%). Even if vb grows
        // to 1_000_000 within the window, the cap stays 1_000.
        let mut d = fresh(0);
        let pct = 1_000;
        let r1 = check_and_advance(&mut d, pct, 600, 600, VB, 0).unwrap().unwrap();
        assert_eq!(r1.max_selloff_cap, 1_000);
        // vb now "grown" to 1_000_000; same window (elapsed < period).
        let r2 = check_and_advance(&mut d, pct, 600, 400, 1_000_000, 100).unwrap().unwrap();
        assert_eq!(r2.max_selloff_cap, 1_000, "cap must use the snapshot, not live vb");
        assert_eq!(r2.vb_snapshot, VB);
        // accumulated 600 + 400 = 1_000 == cap; one more unit fails.
        let err = check_and_advance(&mut d, pct, 600, 1, 1_000_000, 200).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffExceeded"));
    }

    #[test]
    fn new_window_rebaselines_snapshot() {
        // Window 1 snapshots vb=10_000. After one period the window rotates
        // and re-snapshots the (now larger) vb → larger cap.
        let mut d = fresh(0);
        let pct = 1_000; // 10%
        check_and_advance(&mut d, pct, 60, 1_000, VB, 0).unwrap();
        assert_eq!(d.selloff_vb_snapshot, VB);
        // now = 70 → single rotation; vb grew to 20_000 → new cap = 2_000.
        let r = check_and_advance(&mut d, pct, 60, 100, 20_000, 70).unwrap().unwrap();
        assert_eq!(r.vb_snapshot, 20_000);
        assert_eq!(r.max_selloff_cap, 2_000);
        assert_eq!(d.window_start_timestamp, 60);
    }

    #[test]
    fn single_boundary_rotation() {
        let mut d = fresh(0);
        // Seed via a real check so the snapshot is captured.
        check_and_advance(&mut d, FULL, 60, 500, VB, 0).unwrap();
        // now = 70 → elapsed = 70 > period(60), < 2*period.
        let r = check_and_advance(&mut d, FULL, 60, 100, VB, 70).unwrap().unwrap();
        // After rotation: previous = 500, current = 100, window_start = 60.
        assert_eq!(d.previous_selloff, 500);
        assert_eq!(d.current_selloff, 100);
        assert_eq!(d.window_start_timestamp, 60);
        // elapsed_in_window = 70 - 60 = 10, weighted_prev = 500*(60-10)/60 = 416,
        // effective = 416 + 0 + 100 = 516.
        assert_eq!(r.effective_selloff, 516);
    }

    #[test]
    fn double_boundary_clears() {
        let mut d = fresh(0);
        check_and_advance(&mut d, FULL, 60, 500, VB, 0).unwrap();
        d.previous_selloff = 700;
        // elapsed = 200, period = 60 → > 2*60.
        let r = check_and_advance(&mut d, FULL, 60, 100, VB, 200).unwrap().unwrap();
        assert_eq!(d.previous_selloff, 0);
        assert_eq!(d.current_selloff, 100);
        assert_eq!(d.window_start_timestamp, 200);
        assert_eq!(r.effective_selloff, 100);
    }

    #[test]
    fn clock_skew_clamps_to_zero() {
        let mut d = fresh(100);
        // now < window_start (skew). Should not rotate or under-count.
        let r = check_and_advance(&mut d, FULL, 60, 50, VB, 80).unwrap().unwrap();
        assert_eq!(d.window_start_timestamp, 100);
        assert_eq!(d.current_selloff, 50);
        assert_eq!(r.effective_selloff, 50);
    }

    #[test]
    fn invalid_config_period_zero() {
        let mut d = fresh(0);
        let err = check_and_advance(&mut d, FULL, 0, 10, VB, 0).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffInvalidConfig"));
    }

    #[test]
    fn rejects_pct_above_scale() {
        let mut d = fresh(0);
        let err = check_and_advance(&mut d, PERCENT_SCALE + 1, 60, 1, VB, 0).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffInvalidConfig"));
    }

    #[test]
    fn rejects_exact_boundary_overflow() {
        // current at cap; amount_in = 1 must reject (inclusive ceiling).
        let mut d = fresh(0);
        check_and_advance(&mut d, FULL, 60, VB, VB, 0).unwrap(); // fill to cap
        let err = check_and_advance(&mut d, FULL, 60, 1, VB, 1).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffExceeded"));
        assert_eq!(d.current_selloff, VB, "state must not mutate on reject");
    }

    #[test]
    fn tighten_pct_blocks_existing_accumulator() {
        // Admin tightens pct mid-window. Next swap immediately sees the
        // lower ceiling against the same snapshot.
        let mut d = fresh(0);
        check_and_advance(&mut d, FULL, 60, 8_000, VB, 0).unwrap(); // cap 10_000, used 8_000
        // pct now 5_000 (50% of snapshot 10_000 = cap 5_000) < used 8_000.
        let err = check_and_advance(&mut d, 5_000, 60, 1, VB, 10).unwrap_err();
        assert!(format!("{err:?}").contains("MaxSelloffExceeded"));
        assert_eq!(d.current_selloff, 8_000);
    }

    #[test]
    fn period_far_in_future_resets_cleanly() {
        let mut d = fresh(0);
        check_and_advance(&mut d, FULL, 60, 5_000, VB, 0).unwrap();
        d.previous_selloff = u64::MAX / 2;
        d.current_selloff = u64::MAX / 2;
        let r = check_and_advance(&mut d, FULL, 60, 100, VB, i64::MAX / 4).unwrap().unwrap();
        assert_eq!(d.previous_selloff, 0);
        assert_eq!(d.current_selloff, 100);
        assert_eq!(d.window_start_timestamp, i64::MAX / 4);
        assert_eq!(r.effective_selloff, 100);
    }
}

// PORTED VERBATIM from the Coffer contract's `src/math/surge_fee.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! Per-token variable sell-off **surge fee** curve.
//!
//! As a token's max-selloff window fills, swaps selling that token pay a
//! rising surge fee, charged on `amount_out` and routed 100% to the protocol
//! bucket (never LPs).
//!
//! ## Shape: three points, two straight segments
//!
//! The same shape lending protocols use for borrow rates — Evaa's
//! `contracts/logic/master-utils.fc`, and Aave/Compound before it: gentle
//! while the window fills normally, steep once it is nearly exhausted.
//!
//! ```text
//!   fill <= threshold                 →  0            (most trades pay nothing)
//!   threshold .. kink                 →  low  → mid   (linear)
//!   kink .. 100%                      →  mid  → high  (linear, steeper)
//! ```
//!
//! `low`, `mid` and `high` are the fee VALUES at the three points, in
//! `PERCENT_SCALE` units — not slopes. That is the difference from Evaa,
//! which stores slopes; storing values means an operator configures "what fee
//! at full window" directly instead of deriving it.
//!
//! `kink == 0` means "no kink": the curve degenerates to a single straight
//! line from `low` at the threshold to `high` at full fill. Every pool written
//! before the kink existed decodes that way, so they keep a well-defined shape
//! without migration.
//!
//! ## Why linear rather than the exponential this replaced
//!
//! Not because a straight line is inherently fairer — because it is **exact
//! and cheap**. The integral of a straight line is a trapezoid: pure
//! arithmetic, no `LogExpMath::pow`. The convex form needed two `pow` calls
//! per segment just to integrate the rate, which capped the whole scheme at
//! one segment before a swap exceeded the default 200_000 CU budget. Removing
//! them buys segments, and segments are what actually control the error.
//!
//! ## Why the rate is integrated, not sampled
//!
//! A swap does not sit at one rate — it *traverses* a range of them. Sampling
//! at the endpoint and charging the whole output made the fee path-dependent
//! (audit M-05): with a curve charging 0% to 80% fill and 30% at full, one
//! sell of 100 paid 30 while 80 + 20 paid 0 + 6. So the charge is the average
//! rate over the interval the swap actually traverses:
//!
//! ```text
//!   avg = (1 / (f1 - f0)) * ∫[f0..f1] rate(f) df
//! ```
//!
//! For `t` beyond the kink the integral is Evaa's own decomposition — the
//! WHOLE first segment plus the partial second:
//!
//! ```text
//!   F(t) = k·(low + mid)/2  +  mid·(t - k)  +  (high - mid)·(t - k)²/(2·(1 - k))
//! ```
//!
//! The fee rounds UP so the protective fee never undercharges.
//!
//! ## What integration alone does NOT fix
//!
//! The window advances in INPUT units while the fee is charged on OUTPUT, and
//! the AMM curve is concave, so one slice of window is not a proportional
//! slice of output. `swap::handler` closes that separately by charging each
//! segment's real output at that segment's own rate; see
//! `constants::SURGE_FEE_SEGMENTS`.

use crate::coffer::prelude::*;

use crate::coffer::constants::{ONE, PERCENT_SCALE};
use crate::coffer::errors::ErrorCode;

/// Window fill ratio in `PERCENT_SCALE` units, saturating at 100%.
fn fill_ratio(effective_selloff: u64, cap: u64) -> Result<u128> {
    let scale = PERCENT_SCALE as u128;
    Ok(core::cmp::min(
        (effective_selloff as u128)
            .checked_mul(scale)
            .ok_or(ErrorCode::MathOverflow)?
            / (cap as u128),
        scale,
    ))
}

/// Normalise the stored kink into the taxed span.
///
/// `kink_pct` is WHOLE PERCENT of window fill (0..=100) — one byte was all
/// `AssetConfig` had left. It is mapped onto `t ∈ [0, ONE]`, the position
/// within `[threshold, 100%]`.
///
/// Returns `0` — meaning "no kink, one straight line" — whenever the kink
/// would not actually split the taxed span: unset, at or below the threshold,
/// or at or above full fill. Those are exactly the cases where a two-segment
/// curve degenerates, and treating them as a single line keeps the arithmetic
/// free of division by zero.
fn kink_t(kink_pct: u8, threshold_pct: u16) -> u128 {
    let scale = PERCENT_SCALE as u128;
    let thr = threshold_pct as u128;
    if kink_pct == 0 || thr >= scale {
        return 0;
    }
    // whole percent → PERCENT_SCALE units
    let kink_fill = (kink_pct as u128) * (scale / 100);
    if kink_fill <= thr || kink_fill >= scale {
        return 0;
    }
    (kink_fill - thr) * ONE / (scale - thr)
}

/// `fee(t)` — the instantaneous rate at normalised progress `t ∈ [0, ONE]`,
/// in `PERCENT_SCALE` units.
///
/// `saturating_sub` on every span is deliberate: a mis-ordered config
/// (`mid < low`, or `high < mid`) yields a FLAT segment rather than a negative
/// one. `set_max_selloff` rejects such configs outright, so this is
/// defence-in-depth against a config that predates the check or arrives by
/// some other path — the fee can never go negative or wrap.
fn rate_at(
    t_fp: u128,
    slope_low_pct: u16,
    slope_mid_pct: u16,
    slope_high_pct: u16,
    k_fp: u128,
) -> Result<u64> {
    let t = t_fp.min(ONE);
    let (base, span, num, den) = if k_fp == 0 {
        // Single straight line: low → high across the whole taxed span.
        (slope_low_pct, slope_high_pct.saturating_sub(slope_low_pct), t, ONE)
    } else if t <= k_fp {
        (slope_low_pct, slope_mid_pct.saturating_sub(slope_low_pct), t, k_fp)
    } else {
        (
            slope_mid_pct,
            slope_high_pct.saturating_sub(slope_mid_pct),
            t - k_fp,
            ONE - k_fp,
        )
    };

    let prod = (span as u128).checked_mul(num).ok_or(ErrorCode::MathOverflow)?;
    // Round the spanned portion UP — the protective fee never undercharges.
    let add = prod / den + if prod % den != 0 { 1 } else { 0 };
    Ok((base as u128)
        .checked_add(add)
        .ok_or(ErrorCode::MathOverflow)?
        .min(PERCENT_SCALE as u128) as u64)
}

/// `F(t) = ∫[0..t] fee(u) du`, in units of `PERCENT_SCALE × 1e18`.
///
/// Trapezoidal and therefore EXACT for a piecewise-linear rate — this is not
/// a quadrature approximation. Past the kink it is Evaa's decomposition: the
/// whole first segment, then the partial second.
fn integral_to(
    t_fp: u128,
    slope_low_pct: u16,
    slope_mid_pct: u16,
    slope_high_pct: u16,
    k_fp: u128,
) -> Result<u128> {
    let t = t_fp.min(ONE);
    let low = slope_low_pct as u128;
    let mid = slope_mid_pct as u128;

    // ∫[0..x] of a line rising by `span` over width `den`, starting at 0:
    //   span · x² / (2 · den)
    // Ordered so nothing overflows: x² is at most 1e36, and the division
    // happens before multiplying by the span.
    let ramp = |x: u128, span: u128, den: u128| -> Result<u128> {
        if den == 0 || span == 0 || x == 0 {
            return Ok(0);
        }
        let half_sq = x.checked_mul(x).ok_or(ErrorCode::MathOverflow)? / (2 * den);
        span.checked_mul(half_sq).ok_or_else(|| ErrorCode::MathOverflow.into())
    };

    if k_fp == 0 {
        let flat = low.checked_mul(t).ok_or(ErrorCode::MathOverflow)?;
        let rise = ramp(t, (slope_high_pct.saturating_sub(slope_low_pct)) as u128, ONE)?;
        return flat.checked_add(rise).ok_or_else(|| ErrorCode::MathOverflow.into());
    }

    if t <= k_fp {
        let flat = low.checked_mul(t).ok_or(ErrorCode::MathOverflow)?;
        let rise = ramp(t, (slope_mid_pct.saturating_sub(slope_low_pct)) as u128, k_fp)?;
        return flat.checked_add(rise).ok_or_else(|| ErrorCode::MathOverflow.into());
    }

    // Whole first segment: the trapezoid k · (low + mid) / 2.
    let first = k_fp
        .checked_mul(low + mid)
        .ok_or(ErrorCode::MathOverflow)?
        / 2;
    // Partial second segment.
    let rest = t - k_fp;
    let flat = mid.checked_mul(rest).ok_or(ErrorCode::MathOverflow)?;
    let rise = ramp(
        rest,
        (slope_high_pct.saturating_sub(slope_mid_pct)) as u128,
        ONE - k_fp,
    )?;
    first
        .checked_add(flat)
        .and_then(|v| v.checked_add(rise))
        .ok_or_else(|| ErrorCode::MathOverflow.into())
}

/// Average surge fee percentage (in `PERCENT_SCALE` units,
/// `0..=PERCENT_SCALE`) for a swap that moved the input token's window from
/// `effective_selloff_before` to `effective_selloff_after` against `cap`.
///
/// Returns `0` (no surge) when disabled: `cap == 0`, `slope_high_pct == 0`,
/// `threshold_pct >= PERCENT_SCALE`, or the whole traversed range sits below
/// `threshold_pct`.
///
/// # Errors
/// `MathOverflow` from the fixed-point pow path (unreachable for valid
/// inputs; defence-in-depth).
pub fn calc_surge_fee_pct(
    effective_selloff_before: u64,
    effective_selloff_after: u64,
    cap: u64,
    threshold_pct: u16,
    slope_low_pct: u16,
    slope_mid_pct: u16,
    slope_high_pct: u16,
    kink_pct: u8,
) -> Result<u64> {
    // Disabled / no curve.
    if cap == 0 || slope_high_pct == 0 {
        return Ok(0);
    }
    let scale = PERCENT_SCALE as u128;
    let thr = threshold_pct as u128;
    // threshold must be < SCALE (denominator below). If mis-set, never fire.
    if thr >= scale {
        return Ok(0);
    }

    let k_fp = kink_t(kink_pct, threshold_pct);

    let f0 = fill_ratio(effective_selloff_before, cap)?;
    let f1 = fill_ratio(effective_selloff_after, cap)?;

    // Nothing of this swap lands in the surge zone.
    if f1 <= thr {
        return Ok(0);
    }

    let denom = scale - thr;
    let t1 = (f1 - thr)
        .checked_mul(ONE)
        .ok_or(ErrorCode::MathOverflow)?
        / denom;

    // Degenerate interval — the swap did not advance the fill (both fills
    // saturated at 100%, or the amount rounded away). Fall back to the
    // instantaneous rate at the endpoint, which is the conservative choice.
    if f1 <= f0 {
        return rate_at(t1, slope_low_pct, slope_mid_pct, slope_high_pct, k_fp);
    }

    let f0_clamped = core::cmp::max(f0, thr);
    let t0 = (f0_clamped - thr)
        .checked_mul(ONE)
        .ok_or(ErrorCode::MathOverflow)?
        / denom;

    let integral = integral_to(t1, slope_low_pct, slope_mid_pct, slope_high_pct, k_fp)?
        .saturating_sub(integral_to(t0, slope_low_pct, slope_mid_pct, slope_high_pct, k_fp)?);

    // avg = (SCALE - thr) · ∫fee dt / (f1 - f0_clamped), then drop the 1e18
    // scale that `t` carries. Rounded UP: the protective fee never
    // undercharges.
    //
    // The width is the TAXED span, not the whole swap. Averaging over the
    // whole swap and applying the result to the whole output was the bulk of
    // M-05's path-dependence: the part of the swap that ran below the
    // threshold owes nothing, yet it diluted the rate and was then charged
    // anyway. The caller pairs this rate with the output produced above the
    // threshold — see `swap::handler`.
    let numerator = denom
        .checked_mul(integral)
        .ok_or(ErrorCode::MathOverflow)?;
    let width = f1 - f0_clamped;
    let scaled_den = width.checked_mul(ONE).ok_or(ErrorCode::MathOverflow)?;
    let avg = numerator / scaled_den + if numerator % scaled_den != 0 { 1 } else { 0 };

    Ok(avg.min(scale) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALE: u64 = PERCENT_SCALE as u64; // 10_000

    #[test]
    fn disabled_when_cap_zero() {
        assert_eq!(calc_surge_fee_pct(0, 100, 0, 8000, 200, 200, 3000, 0).unwrap(), 0);
    }

    #[test]
    fn disabled_when_slope_high_zero() {
        assert_eq!(calc_surge_fee_pct(0, 100, 100, 8000, 0, 0, 0, 0).unwrap(), 0);
    }

    #[test]
    fn zero_below_threshold() {
        // whole range under 80% fill → 0.
        assert_eq!(calc_surge_fee_pct(0, 50, 100, 8000, 200, 200, 3000, 0).unwrap(), 0);
    }

    #[test]
    fn full_sweep_is_the_curve_average_not_the_endpoint() {
        // 0 → 100% fill. The endpoint rate is slope_high (3000); the AVERAGE
        // over the traversal is strictly lower.
        let avg = calc_surge_fee_pct(0, 10_000, 10_000, 8000, 0, 0, 3000, 0).unwrap();
        assert!(avg > 0 && avg < 3000, "avg over full sweep = {avg}");
    }

    /// The property the whole rewrite exists for: splitting a sell must not
    /// reduce the fee. Charged fee is `avg × width`, so we compare integrals.
    /// Simulate the REAL charge — taxed output through the AMM curve —
    /// rather than `rate × window-width`.
    ///
    /// The previous version of this test multiplied the average rate by
    /// `(to - from)`, i.e. by a width measured in INPUT-window units, while
    /// production multiplies by OUTPUT. It therefore proved that the integral
    /// is additive in its own units — which it is — and said nothing about the
    /// property that actually matters. It stayed green while a single
    /// full-window sell was charged 1.88× what the same sell paid when split.
    fn simulate(parts: &[(u64, u64)], cap: u64, thr: u16, low: u16, high: u16) -> u128 {
        use crate::coffer::math::CofferMath;
        // Equal-weight pool, so output is concave in input — the property the
        // old test's linear model quietly assumed away.
        const VB: u64 = 1_000_000;
        const W: u64 = 5_000;
        let mut vb_in = VB;
        let mut vb_out = VB;
        let mut total_fee: u128 = 0;

        for &(from, to) in parts {
            let amount_in = to - from;
            let amount_out = CofferMath::calc_out_given_in(
                vb_in, W, vb_out, W, amount_in, vb_out, 9, 9,
            )
            .unwrap();

            let avg = calc_surge_fee_pct(from, to, cap, thr, low, low, high, 0).unwrap() as u128;
            // Mirror `swap::handler`: only the output produced ABOVE the
            // threshold is charged.
            let thr_units = (cap as u128) * (thr as u128) / 10_000;
            let taxed_out = if (from as u128) >= thr_units {
                amount_out
            } else if (to as u128) <= thr_units {
                0
            } else {
                let untaxed_span = thr_units - from as u128;
                let x_untaxed = (amount_in as u128) * untaxed_span / (amount_in as u128);
                let x_untaxed = (x_untaxed.min(untaxed_span)) as u64;
                let y_untaxed = CofferMath::calc_out_given_in(
                    vb_in, W, vb_out, W, x_untaxed, vb_out, 9, 9,
                )
                .unwrap();
                amount_out.saturating_sub(y_untaxed)
            };
            total_fee += (taxed_out as u128) * avg / 10_000;

            vb_in += amount_in;
            vb_out -= amount_out;
        }
        total_fee
    }

    /// The charge must not depend materially on how the sell is sliced.
    ///
    /// A residual spread remains by design: charging the taxed output at the
    /// taxed span's AVERAGE rate over-estimates the exact output-weighted
    /// integral, and by more when the span is traversed in one step. The
    /// bound below is the measured worst case for this curve; splitting the
    /// taxed span finer would tighten it at one extra curve evaluation each.
    #[test]
    fn splitting_a_sell_does_not_materially_reduce_the_fee() {
        let cap = 1_000u64;
        let (thr, low, high) = (8000u16, 0u16, 3000u16);

        let whole = simulate(&[(0, 1000)], cap, thr, low, high);
        let cases: [(&str, u128); 3] = [
            ("800+200", simulate(&[(0, 800), (800, 1000)], cap, thr, low, high)),
            (
                "800+100+100",
                simulate(&[(0, 800), (800, 900), (900, 1000)], cap, thr, low, high),
            ),
            (
                "5×200",
                simulate(
                    &[(0, 200), (200, 400), (400, 600), (600, 800), (800, 1000)],
                    cap,
                    thr,
                    low,
                    high,
                ),
            ),
        ];
        for (label, split) in cases {
            assert!(split > 0, "split {label} paid nothing");
            let ratio = whole as f64 / split as f64;
            assert!(
                ratio <= 1.10,
                "split {label} vs whole {whole}, ratio too high"
            );
        }
    }

    /// The exact defect the audit reported, in production units: under
    /// endpoint sampling the 80+20 split paid five times less than the whole
    /// sell. Pin that the gap is now a few percent, not a multiple.
    #[test]
    fn regression_audit_m05_split_dodge_closed() {
        let cap = 1_000u64;
        let (thr, low, high) = (8000u16, 0u16, 3000u16);
        let whole = simulate(&[(0, 1000)], cap, thr, low, high);
        let split = simulate(&[(0, 800), (800, 1000)], cap, thr, low, high);
        assert!(
            whole as f64 / split as f64 <= 1.10,
            "whole {whole} vs split {split} — the dodge is back"
        );
    }

    /// Splitting must never make the fee LARGER either — an asymmetry would
    /// be just as exploitable in reverse (grief a competitor's routing).
    #[test]
    fn neg_splitting_cannot_inflate_the_fee_either() {
        let cap = 1_000u64;
        let (thr, low, high) = (8000u16, 0u16, 3000u16);
        let whole = simulate(&[(0, 1000)], cap, thr, low, high);
        let split = simulate(&[(0, 800), (800, 1000)], cap, thr, low, high);
        assert!(split <= whole, "splitting cost MORE: {split} vs {whole}");
    }

    /// Port of the auditor's own PoC (`audit/serokell/poc/audit_v5_poc.rs`).
    ///
    /// Their reproduction was `single == 30`, `split == 6`, i.e. the whole
    /// sell cost five times what the split one did. The assertion to keep
    /// alive is that no split shape is anywhere near five times cheaper.
    ///
    /// It is computed through `simulate`, which charges the way `swap::handler`
    /// does — taxed OUTPUT at the taxed span's rate. An earlier version of this
    /// test multiplied the rate by `(to - from)`, a width in INPUT-window
    /// units, and that category error is precisely the one M-05's first fix
    /// left behind: it made the test agree with a charge nobody computes.
    #[test]
    fn auditor_poc_m05_numbers() {
        let cap = 1_000u64;
        let (thr, lo, hi) = (8000u16, 0u16, 3000u16);

        let single = simulate(&[(0, 1000)], cap, thr, lo, hi);
        let two_way = simulate(&[(0, 800), (800, 1000)], cap, thr, lo, hi);
        let three_way = simulate(&[(0, 800), (800, 900), (900, 1000)], cap, thr, lo, hi);
        let fifths = simulate(
            &[(0, 200), (200, 400), (400, 600), (600, 800), (800, 1000)],
            cap,
            thr,
            lo,
            hi,
        );

        println!(
            "M-05 after fix: single={single} 800+200={two_way} \
             800+100+100={three_way} 5x200={fifths}"
        );

        for (label, split) in [
            ("800+200", two_way),
            ("800+100+100", three_way),
            ("5x200", fifths),
        ] {
            assert!(split > 0, "{label} paid nothing");
            assert!(
                single < split * 5,
                "the auditor's 5x-cheaper dodge still reproduces for {label}: \
                 single={single} split={split}"
            );
        }
    }

    #[test]
    fn monotonic_in_end_fill() {
        let mut last = 0u64;
        for fill in [8001u64, 8500, 9000, 9500, 9900, 10_000] {
            let f = calc_surge_fee_pct(8000, fill, 10_000, 8000, 200, 200, 3000, 0).unwrap();
            assert!(f >= last, "fee dropped at fill {fill}: {f} < {last}");
            last = f;
        }
    }

    #[test]
    fn saturates_above_full_fill() {
        let f = calc_surge_fee_pct(10_000, 99_999, 10_000, 8000, 200, 200, 3000, 0).unwrap();
        assert!(f <= SCALE);
    }

    #[test]
    fn fee_never_exceeds_scale() {
        let f = calc_surge_fee_pct(0, 10_000, 10_000, 0, 9000, 9000, SCALE as u16, 0).unwrap();
        assert!(f <= SCALE);
    }

    #[test]
    fn degenerate_interval_uses_point_rate() {
        // before == after, both at full fill → instantaneous rate = high.
        let f = calc_surge_fee_pct(10_000, 10_000, 10_000, 8000, 200, 200, 3000, 0).unwrap();
        assert_eq!(f, 3000);
    }

    #[test]
    fn threshold_zero_taxes_from_start() {
        let f = calc_surge_fee_pct(0, 10_000, 10_000, 0, 0, 0, 10_000, 0).unwrap();
        assert!(f > 0 && f < 10_000, "average over [0,1] of a convex curve: {f}");
    }
}

/// The three-point curve, attacked from every direction a config can be bent.
///
/// The threat these pin down: an operator (or a bug) setting a shape that
/// makes the protective fee smaller as the window gets FULLER, or that
/// produces a negative/wrapped value, or that divides by a zero-width
/// segment. `set_max_selloff` rejects such configs, but the math must be safe
/// even if one reaches it — a rejected instruction is one layer, not two.
#[cfg(test)]
mod curve_shape_tests {
    use super::*;

    const SCALE: u64 = PERCENT_SCALE as u64;

    /// Rate at a given window fill, going through the public entry point via
    /// the degenerate-interval branch (`before == after`).
    fn rate(fill: u64, cap: u64, thr: u16, lo: u16, mid: u16, hi: u16, kink: u8) -> u64 {
        calc_surge_fee_pct(fill, fill, cap, thr, lo, mid, hi, kink).unwrap()
    }

    // ── Shape: the curve does what three points should ──────────────────

    /// The fee passes through all three configured points.
    ///
    /// Sampled just ABOVE the threshold, because the threshold itself is
    /// untaxed — see `pos_threshold_is_a_step_not_a_ramp`.
    #[test]
    fn pos_curve_hits_its_three_points() {
        let (cap, thr, lo, mid, hi, kink) = (10_000u64, 2_000u16, 100u16, 500u16, 5_000u16, 60u8);
        // `lo + 1`: the spanned portion is rounded UP, so one unit past the
        // threshold already carries a whole unit of the ramp. That is the
        // deliberate "never undercharge" direction, not drift.
        assert_eq!(rate(2_001, cap, thr, lo, mid, hi, kink), lo as u64 + 1, "just past the threshold");
        assert_eq!(rate(6_000, cap, thr, lo, mid, hi, kink), mid as u64, "at the kink");
        assert_eq!(rate(10_000, cap, thr, lo, mid, hi, kink), hi as u64, "at full fill");
    }

    /// The threshold is a STEP, not the start of a ramp: at exactly the
    /// threshold the fee is 0, and one unit above it the fee is already
    /// `low`. Pinned because it is easy to misread the curve as continuous
    /// and then be surprised by a jump — and because a `low > 0` config makes
    /// that jump the difference between paying nothing and paying `low` on a
    /// single extra unit of fill.
    #[test]
    fn pos_threshold_is_a_step_not_a_ramp() {
        let (cap, thr, lo, mid, hi, kink) = (10_000u64, 3_000u16, 250u16, 800u16, 4_000u16, 70u8);
        assert_eq!(rate(3_000, cap, thr, lo, mid, hi, kink), 0, "at the threshold: untaxed");
        assert_eq!(
            rate(3_001, cap, thr, lo, mid, hi, kink),
            lo as u64 + 1,
            "one unit above: low, plus the deliberate round-up of the ramp"
        );
    }

    /// Between the points it is a straight line, not a curve: the midpoint of
    /// each segment sits at the arithmetic mean of its ends.
    #[test]
    fn pos_each_segment_is_linear() {
        let (cap, thr, lo, mid, hi, kink) = (10_000u64, 0u16, 0u16, 400u16, 2_000u16, 50u8);
        // halfway up the first segment: fill 2500 → (0 + 400)/2 = 200
        assert_eq!(rate(2_500, cap, thr, lo, mid, hi, kink), 200);
        // halfway up the second: fill 7500 → (400 + 2000)/2 = 1200
        assert_eq!(rate(7_500, cap, thr, lo, mid, hi, kink), 1_200);
    }

    /// The fee is non-decreasing in window fill for every legal shape. This is
    /// the property the whole mechanism rests on: selling later must never be
    /// cheaper than selling earlier.
    #[test]
    fn pos_rate_is_monotonic_in_fill_for_every_legal_shape() {
        for (thr, lo, mid, hi, kink) in [
            (0u16, 0u16, 0u16, 10_000u16, 50u8),
            (2_000, 100, 500, 5_000, 60),
            (8_000, 0, 0, 3_000, 90),
            (5_000, 1_000, 1_000, 1_000, 75), // flat
            (0, 0, 9_999, 10_000, 1),
            (0, 0, 1, 10_000, 99),
        ] {
            let mut last = 0u64;
            for fill in (0..=10_000).step_by(97) {
                let r = rate(fill, 10_000, thr, lo, mid, hi, kink);
                assert!(
                    r >= last,
                    "fee FELL at fill {fill} for ({thr},{lo},{mid},{hi},{kink}): {r} < {last}"
                );
                last = r;
            }
        }
    }

    /// A kink of 0 means "no kink": one straight line from `low` to `high`,
    /// with `mid` ignored entirely. This is what every pool written before the
    /// kink existed decodes as, so it must be well-defined.
    #[test]
    fn pos_zero_kink_is_a_single_line_and_ignores_mid() {
        let (cap, thr, lo, hi) = (10_000u64, 0u16, 0u16, 1_000u16);
        for junk_mid in [0u16, 1, 500, 9_999, u16::MAX] {
            assert_eq!(rate(5_000, cap, thr, lo, junk_mid, hi, 0), 500, "mid must not matter");
        }
    }

    // ── Negative: shapes that would invert or break the brake ────────────

    /// `mid < low` must not produce a fee below `low`. The math clamps the
    /// segment flat instead of letting the span go negative and wrap.
    #[test]
    fn neg_mid_below_low_never_dips_under_low() {
        let (cap, thr, lo, hi, kink) = (10_000u64, 0u16, 5_000u16, 8_000u16, 50u8);
        for bad_mid in [0u16, 1, 100, 4_999] {
            // strictly above the threshold — at or below it the fee is 0 by
            // design, which is not a "dip".
            for fill in [1u64, 1_000, 2_500, 4_999, 5_000] {
                let r = rate(fill, cap, thr, lo, bad_mid, hi, kink);
                assert!(r >= lo as u64, "fee {r} dipped below low {lo} at fill {fill}");
            }
        }
    }

    /// `high < mid` must not produce a fee that falls after the kink.
    #[test]
    fn neg_high_below_mid_never_falls_after_the_kink() {
        let (cap, thr, lo, mid, kink) = (10_000u64, 0u16, 0u16, 6_000u16, 50u8);
        for bad_hi in [0u16, 1, 100, 5_999] {
            let at_kink = rate(5_000, cap, thr, lo, mid, bad_hi, kink);
            for fill in [5_001u64, 6_000, 8_000, 10_000] {
                let r = rate(fill, cap, thr, lo, mid, bad_hi, kink);
                assert!(
                    r >= at_kink,
                    "fee fell from {at_kink} to {r} after the kink with high={bad_hi}"
                );
            }
        }
    }

    /// Every legal and illegal shape stays inside `[0, PERCENT_SCALE]`. A fee
    /// above 100% of the output would underflow the user's payout.
    #[test]
    fn neg_rate_never_exceeds_one_hundred_percent() {
        for (thr, lo, mid, hi, kink) in [
            (0u16, u16::MAX, u16::MAX, u16::MAX, 50u8),
            (0, 0, u16::MAX, u16::MAX, 1),
            (0, 9_000, 9_500, u16::MAX, 99),
            (9_999, 10_000, 10_000, 10_000, 0),
        ] {
            for fill in (0..=10_000).step_by(313) {
                let r = rate(fill, 10_000, thr, lo, mid, hi, kink);
                assert!(r <= SCALE, "fee {r} exceeded PERCENT_SCALE at fill {fill}");
            }
        }
    }

    /// A kink at or below the threshold would give the first segment zero
    /// width — a division by zero if it reached the math. It is treated as
    /// "no kink" instead.
    #[test]
    fn neg_kink_at_or_below_threshold_degenerates_safely() {
        let (cap, thr, lo, mid, hi) = (10_000u64, 5_000u16, 0u16, 9_000u16, 1_000u16);
        for bad_kink in [1u8, 10, 25, 50] {
            // kink_fill = bad_kink * 100 <= thr = 5000
            for fill in (0..=10_000).step_by(499) {
                let r = rate(fill, cap, thr, lo, mid, hi, bad_kink);
                assert!(r <= SCALE);
            }
            // …and it behaves as the no-kink line, so `mid` is ignored.
            assert_eq!(
                rate(7_500, cap, thr, lo, mid, hi, bad_kink),
                rate(7_500, cap, thr, lo, 0, hi, 0),
            );
        }
    }

    /// A kink at 100% would give the SECOND segment zero width. Same
    /// treatment.
    #[test]
    fn neg_kink_at_full_fill_degenerates_safely() {
        let (cap, thr, lo, mid, hi) = (10_000u64, 0u16, 0u16, 500u16, 1_000u16);
        for fill in (0..=10_000).step_by(499) {
            assert!(rate(fill, cap, thr, lo, mid, hi, 100) <= SCALE);
        }
        assert_eq!(
            rate(5_000, cap, thr, lo, mid, hi, 100),
            rate(5_000, cap, thr, lo, 0, hi, 0),
        );
    }

    /// A kink above 100 is out of range for the axis it lives on. It must not
    /// wrap into a valid-looking position.
    #[test]
    fn neg_kink_above_one_hundred_does_not_wrap() {
        let (cap, thr, lo, mid, hi) = (10_000u64, 0u16, 0u16, 500u16, 1_000u16);
        for bad in [101u8, 128, 200, 255] {
            for fill in (0..=10_000).step_by(997) {
                assert!(rate(fill, cap, thr, lo, mid, hi, bad) <= SCALE);
            }
        }
    }

    /// A threshold at or above full fill leaves no taxed zone at all.
    #[test]
    fn neg_threshold_at_or_above_scale_never_charges() {
        for thr in [10_000u16, 10_001, 20_000, u16::MAX] {
            for fill in [0u64, 5_000, 9_999, 10_000] {
                assert_eq!(rate(fill, 10_000, thr, 0, 5_000, 10_000, 50), 0);
            }
        }
    }

    /// A zero cap disables the window, and with it the fee — no division by
    /// zero on the fill ratio.
    #[test]
    fn neg_zero_cap_charges_nothing() {
        for fill in [0u64, 1, u64::MAX] {
            assert_eq!(rate(fill, 0, 0, 0, 5_000, 10_000, 50), 0);
        }
    }

    /// Fill beyond the cap saturates at 100% rather than running the curve
    /// past its end.
    #[test]
    fn neg_fill_beyond_cap_saturates() {
        let (cap, thr, lo, mid, hi, kink) = (1_000u64, 0u16, 0u16, 500u16, 1_000u16, 50u8);
        let at_full = rate(1_000, cap, thr, lo, mid, hi, kink);
        for over in [1_001u64, 5_000, u64::MAX / 2] {
            assert_eq!(rate(over, cap, thr, lo, mid, hi, kink), at_full);
        }
    }

    /// Extreme inputs must not overflow the fixed-point arithmetic.
    #[test]
    fn neg_extreme_inputs_do_not_overflow() {
        for &(before, after, cap) in &[
            (0u64, u64::MAX, u64::MAX),
            (u64::MAX - 1, u64::MAX, u64::MAX),
            (0, 1, 1),
            (u64::MAX, u64::MAX, 1),
        ] {
            for kink in [0u8, 1, 50, 99, 100, 255] {
                let r = calc_surge_fee_pct(before, after, cap, 0, 0, 5_000, 10_000, kink);
                assert!(r.is_ok(), "overflowed at ({before},{after},{cap},{kink})");
                assert!(r.unwrap() <= SCALE);
            }
        }
    }

    /// The average over any interval must lie between the rates at its ends —
    /// a mean outside its own bounds would mean the integral is wrong.
    #[test]
    fn pos_average_is_bracketed_by_the_endpoint_rates() {
        let (cap, thr, lo, mid, hi, kink) = (10_000u64, 1_000u16, 200u16, 1_500u16, 9_000u16, 70u8);
        for (a, b) in [(1_000u64, 3_000u64), (3_000, 7_000), (7_000, 10_000), (1_000, 10_000),
                       (6_900, 7_100)] {
            let avg = calc_surge_fee_pct(a, b, cap, thr, lo, mid, hi, kink).unwrap();
            let ra = rate(a, cap, thr, lo, mid, hi, kink);
            let rb = rate(b, cap, thr, lo, mid, hi, kink);
            // +1 of slack for the deliberate round-up of the spanned portion.
            assert!(
                avg + 1 >= ra && avg <= rb + 1,
                "average {avg} outside [{ra}, {rb}] on [{a}, {b}]"
            );
        }
    }

    /// The integral is additive across the kink — the property Evaa's
    /// decomposition provides and the reason the kink may sit anywhere.
    #[test]
    fn pos_integral_is_additive_across_the_kink() {
        let (cap, thr, lo, mid, hi, kink) = (10_000u64, 0u16, 0u16, 1_000u16, 5_000u16, 40u8);
        let whole = calc_surge_fee_pct(0, 10_000, cap, thr, lo, mid, hi, kink).unwrap() as u128
            * 10_000u128;
        let left = calc_surge_fee_pct(0, 4_000, cap, thr, lo, mid, hi, kink).unwrap() as u128
            * 4_000u128;
        let right = calc_surge_fee_pct(4_000, 10_000, cap, thr, lo, mid, hi, kink).unwrap() as u128
            * 6_000u128;
        let sum = left + right;
        let diff = whole.abs_diff(sum);
        assert!(
            diff * 1_000 <= whole,
            "integral not additive across the kink: whole {whole} vs {sum}"
        );
    }
}

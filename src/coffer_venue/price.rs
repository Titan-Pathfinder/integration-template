//! Marginal price `d(user_output) / d(amount_in)` for the weighted curve.
//!
//! With `V_in`, `V_out` the virtual balances, `w_in / w_out` the weights,
//! `phi` the swap-fee fraction and `x' = (1 - phi) x`, the contract's curve is
//!
//! ```text
//! f(x) = V_out * (1 - (V_in / (V_in + x'))^r),   r = w_in / w_out
//! ```
//!
//! whose derivative is
//!
//! ```text
//! f'(x) = (1 - phi) * V_out * r * V_in^r * (V_in + x')^(-r-1)
//!       = (1 - phi) * V_out * r * (V_in / (V_in + x'))^r / (V_in + x')
//! ```
//!
//! This is positive and strictly decreasing in `x` (the curve is concave).
//!
//! The venue only admits the prefix where the contract charges no surge
//! fee (see `domain.rs`). On that domain this is the smooth weighted-curve
//! derivative; integer input-fee and output rounding still apply to the
//! exact output. The four moving surge segments must not be approximated
//! here by multiplying this derivative by an endpoint fee rate.

use crate::coffer::constants::SWAP_FEE_PRECISION;

/// Everything the closed-form derivative needs, all in `f64`.
#[derive(Debug, Clone, Copy)]
pub struct CurveParams {
    pub virtual_balance_in: f64,
    pub virtual_balance_out: f64,
    /// `w_in / w_out`.
    pub weight_ratio: f64,
    /// `1 - swap_fee_rate / SWAP_FEE_PRECISION`.
    pub keep: f64,
}

impl CurveParams {
    pub fn new(
        virtual_balance_in: u64,
        weight_in: u64,
        virtual_balance_out: u64,
        weight_out: u64,
        swap_fee_rate: u32,
    ) -> Self {
        Self {
            virtual_balance_in: virtual_balance_in as f64,
            virtual_balance_out: virtual_balance_out as f64,
            weight_ratio: weight_in as f64 / weight_out as f64,
            keep: 1.0 - swap_fee_rate as f64 / SWAP_FEE_PRECISION as f64,
        }
    }

    /// `f'(amount_in)` of the fee-then-curve output, output atoms per input atom.
    pub fn marginal_output(&self, amount_in: u64) -> f64 {
        let effective_in = self.keep * amount_in as f64;
        let denom = self.virtual_balance_in + effective_in;
        let base = self.virtual_balance_in / denom;
        self.keep * self.virtual_balance_out * self.weight_ratio * base.powf(self.weight_ratio)
            / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coffer::math::CofferMath;

    fn params() -> CurveParams {
        CurveParams::new(5_000_000_000_000, 8_000, 30_000_000_000_000, 2_000, 3_000)
    }

    /// The closed form must agree with a central difference of the exact
    /// integer curve (fee included).
    #[test]
    fn marginal_output_matches_finite_difference_of_the_ported_curve() {
        let p = params();
        for &x in &[
            10_000_000u64,
            1_000_000_000,
            50_000_000_000,
            2_000_000_000_000,
        ] {
            let analytic = p.marginal_output(x);
            let h = (x / 1000).max(100_000);
            let f = |amt: u64| {
                let fee = crate::coffer::swap::calculate_swap_fee(amt, 3_000).unwrap();
                CofferMath::calc_out_given_in(
                    5_000_000_000_000,
                    8_000,
                    30_000_000_000_000,
                    2_000,
                    amt - fee,
                    u64::MAX,
                    9,
                    9,
                )
                .unwrap() as f64
            };
            let fd = (f(x + h) - f(x - h)) / (2.0 * h as f64);
            let rel = (analytic - fd).abs() / fd;
            assert!(rel < 1e-3, "x={x}: analytic={analytic} fd={fd} rel={rel}");
        }
    }

    #[test]
    fn marginal_output_is_positive_and_decreasing() {
        let p = params();
        let mut prev = f64::INFINITY;
        for &x in &[0u64, 1, 1_000, 1_000_000, 1_000_000_000, 1_000_000_000_000] {
            let v = p.marginal_output(x);
            assert!(v > 0.0 && v <= prev, "x={x}: {v} vs {prev}");
            prev = v;
        }
    }
}

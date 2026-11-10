// PORTED VERBATIM from the Coffer contract's `src/math/weighted_math.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! Pool-level invariant + weight validation.
//!
//! Two things live here:
//! 1. `validate_weights`        — guard for `initialize_coffer_pool`.
//! 2. `calculate_invariant`     — `∏ balance_i^weight_i` via `LogExpMath`.

use crate::coffer::constants::*;
use crate::coffer::errors::ErrorCode;
use crate::coffer::math::log_exp_math::{LogExpMath, mul_div_down};
use crate::coffer::prelude::*;

/// Namespace.
pub struct WeightedMath;

impl WeightedMath {
    /// Compute the weighted-pool invariant `I = ∏ balance_i^weight_i`.
    ///
    /// Balances are normalised to a 6-decimal scale internally (`balance
    /// /= 10^(decimals - 6)` for `decimals ≥ 6`, multiplied otherwise) to
    /// keep `LogExpMath::pow` inputs in its well-conditioned range, even
    /// for pools with very large virtual balances. The invariant is used
    /// only as a *relative* anchor (BPT minting on the first deposit,
    /// fee tracking) so the constant scaling factor cancels.
    ///
    /// Returns `0` early if any balance is zero — multiplication by zero
    /// would zero the invariant anyway, but short-circuiting saves CU.
    ///
    /// # Parameters
    /// * `balances`           — raw token balances, length = `weights.len()`.
    /// * `normalized_weights` — basis-point weights summing to `WEIGHT_SCALE`.
    /// * `decimals`           — per-token decimals.
    ///
    /// # Returns
    /// `u128` invariant in 18-decimal fixed-point (relative scale).
    ///
    /// # Errors
    /// `InvalidAmounts`, `InvalidTokenCount`, `MathOverflow` from
    /// `LogExpMath::pow`.
    ///
    /// # Used by
    /// `instructions::add_liquidity` (first deposit, BPT seed).
    pub fn calculate_invariant(
        balances: &[u64],
        normalized_weights: &[u64],
        decimals: &[u8],
    ) -> Result<u128> {
        require!(
            balances.len() == normalized_weights.len(),
            ErrorCode::InvalidArrayLength
        );
        require!(balances.len() >= 2, ErrorCode::InvalidTokenCount);

        let mut invariant = ONE;

        for i in 0..balances.len() {
            let raw_balance = balances[i] as u128;
            if raw_balance == 0 {
                return Ok(0);
            }

            let dec = decimals[i] as u32;
            const TARGET_DEC: u32 = 6;
            let balance = if dec >= TARGET_DEC {
                raw_balance / 10u128.pow(dec - TARGET_DEC).max(1)
            } else {
                raw_balance
                    .checked_mul(10u128.pow(TARGET_DEC - dec))
                    .ok_or(ErrorCode::MathOverflow)?
            };

            if balance == 0 {
                return Ok(0);
            }

            let weight = weight_to_fixed_point(normalized_weights[i]);

            let powered = LogExpMath::pow(balance, weight)?;

            invariant = mul_div_down(invariant, powered, ONE)?;
        }

        Ok(invariant)
    }

    /// Validate normalised weights for `initialize_coffer_pool`.
    ///
    /// Enforces:
    /// - `Σ weights == WEIGHT_SCALE` (= 10_000),
    /// - `MIN_WEIGHT ≤ weights[i] ≤ MAX_WEIGHT` for every token.
    ///
    /// # Errors
    /// `InvalidWeights`.
    ///
    /// # Used by
    /// `instructions::initialize_coffer_pool`.
    pub fn validate_weights(weights: &[u64]) -> Result<()> {
        let sum: u64 = weights.iter().sum();
        require!(sum == WEIGHT_SCALE, ErrorCode::InvalidWeights);

        for &weight in weights {
            require!(
                weight >= MIN_WEIGHT && weight <= MAX_WEIGHT,
                ErrorCode::InvalidWeights
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_weights() {
        let weights = vec![5000u64, 5000u64];
        assert!(WeightedMath::validate_weights(&weights).is_ok());

        let weights_three = vec![3333u64, 3333u64, 3334u64];
        assert!(WeightedMath::validate_weights(&weights_three).is_ok());

        let weights_invalid = vec![4000u64, 4000u64];
        assert!(WeightedMath::validate_weights(&weights_invalid).is_err());

        let weights_too_small = vec![50u64, 9950u64];
        assert!(WeightedMath::validate_weights(&weights_too_small).is_err());
    }

    #[test]
    fn test_calculate_invariant() {
        let balances = vec![1_000_000_000u64, 1_000_000_000u64];
        let weights = vec![5000u64, 5000u64];
        let decimals = vec![9u8, 9u8];

        let result = WeightedMath::calculate_invariant(&balances, &weights, &decimals);
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[test]
    fn test_calculate_invariant_different_decimals() {
        let balances = vec![1_000_000u64, 1_000_000_000u64];
        let weights = vec![5000u64, 5000u64];
        let decimals = vec![6u8, 9u8];

        let result = WeightedMath::calculate_invariant(&balances, &weights, &decimals);
        assert!(result.is_ok());
    }

    #[test]
    fn test_weight_conversion() {
        let weight_bps = 5000u64;
        let weight_fp = weight_to_fixed_point(weight_bps);
        assert_eq!(weight_fp, 500_000_000_000_000_000u128);
    }
}

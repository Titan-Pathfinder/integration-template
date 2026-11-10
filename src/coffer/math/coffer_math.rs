// PORTED VERBATIM from the Coffer contract's `src/math/coffer_math.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! Swap and BPT-mint/burn formulas for the weighted Coffer AMM.
//!
//! Decimal handling: balances are kept in their native token decimals
//! throughout. Ratios and exponents go through 18-decimal fixed-point,
//! and the final result drops back to native units. The invariant is
//! relative-only, so we don't need to scale balances to a common base.

use crate::coffer::constants::*;
use crate::coffer::errors::ErrorCode;
use crate::coffer::math::fixed_point::FixedPoint;
use crate::coffer::math::log_exp_math::LogExpMath;
use crate::coffer::prelude::*;

/// Namespace.
pub struct CofferMath;

impl CofferMath {
    /// Constant-product-with-weights `out_given_in`:
    /// `aO = bO * (1 - (bI / (bI + aI))^(wI / wO))`.
    ///
    /// Capped by `actual_balance_out`: if the math says the trader should
    /// receive more than the LP-accessible balance, the swap reverts
    /// rather than silently capping (which would corrupt the
    /// virtual/actual ratio bookkeeping).
    ///
    /// # Parameters
    /// * `virtual_balance_in`  — pool's virtual balance for the input
    ///   token (raw units).
    /// * `weight_in`           — basis points.
    /// * `virtual_balance_out` — same for the output token.
    /// * `weight_out`          — basis points.
    /// * `amount_in`           — raw units, fee-adjusted by the caller.
    /// * `actual_balance_out`  — vault holdings of the output token; the
    ///   computed `amount_out` must be ≤ this.
    /// * `_decimals_in/out`    — accepted for symmetry with the older
    ///   18-dec scaled implementation; not used (operands stay in
    ///   native decimals).
    ///
    /// # Returns
    /// `amount_out` in `decimals_out` raw units.
    ///
    /// # Errors
    /// `MathOverflow`, `AmountOutExceedsBalance`.
    ///
    /// # Used by
    /// `instructions::swap`. single-token-liquidity no longer replicates this
    /// math — it issues real `swap` CPIs and reads the amounts that actually
    /// arrived, so there is nothing left to keep bit-identical across the
    /// program boundary.
    pub fn calc_out_given_in(
        virtual_balance_in: u64,
        weight_in: u64,
        virtual_balance_out: u64,
        weight_out: u64,
        amount_in: u64,
        actual_balance_out: u64,
        _decimals_in: u8,
        _decimals_out: u8,
    ) -> Result<u64> {
        let weight_in_fp = weight_to_fixed_point(weight_in);
        let weight_out_fp = weight_to_fixed_point(weight_out);

        let balance_in = virtual_balance_in as u128;
        let amt_in = amount_in as u128;
        let denominator = balance_in
            .checked_add(amt_in)
            .ok_or(ErrorCode::MathOverflow)?;

        let base = FixedPoint::div_up(balance_in, denominator)?;
        let exponent = FixedPoint::div_down(weight_in_fp, weight_out_fp)?;
        // `LogExpMath::pow` returns a DOWN-rounded result. Bumping by
        // 1 ULP and clamping at ONE gives us LP-safe rounding: amount_out
        // ends up rounded DOWN in the user's direction, never up.
        let power_raw = LogExpMath::pow(base, exponent)?;
        let power = power_raw.saturating_add(1).min(ONE);
        let complement = FixedPoint::complement(power)?;

        let balance_out = virtual_balance_out as u128;
        let amount_out_calc = FixedPoint::mul_down(balance_out, complement)?;

        let actual_out = actual_balance_out as u128;
        require!(
            amount_out_calc <= actual_out,
            ErrorCode::AmountOutExceedsBalance
        );

        amount_out_calc.try_into().map_err(|_| ErrorCode::MathOverflow.into())
    }

    /// Proportional-join BPT calculation. For each token i, ratio_i =
    /// `amounts_in[i] / actual_balances[i]`; we mint BPT proportional to
    /// the **smallest** ratio (so the pool is never over-credited).
    ///
    /// Tokens with `actual_balances[i] == 0` are skipped when picking the
    /// minimum (sidelined slot behaviour — see `add_liquidity` doc), but
    /// the returned `ratio_min` is later applied by the caller to virtual
    /// balances of ALL tokens so sidelined slots stay synchronised.
    ///
    /// # Parameters
    /// * `actual_balances` — pre-deposit pool balances.
    /// * `amounts_in`      — user's deposit amounts (same indices).
    /// * `_decimals`       — kept for ABI symmetry; unused.
    /// * `bpt_total_supply`— pre-deposit BPT supply.
    ///
    /// # Returns
    /// `(bpt_amount, ratio_min)`:
    /// - `bpt_amount` — BPT to mint (raw `u64`, 9 decimals).
    /// - `ratio_min` — limiting ratio in 18-decimal fixed-point. The
    ///   caller multiplies each virtual balance by this to grow virtuals
    ///   proportionally, including sidelined tokens.
    ///
    /// # Errors
    /// `InvalidAmounts` (length mismatch / no live tokens),
    /// `MathOverflow`.
    ///
    /// # Used by
    /// `instructions::add_liquidity` (subsequent deposits).
    pub fn calc_bpt_out_given_exact_tokens_in(
        actual_balances: &[u64],
        amounts_in: &[u64],
        _decimals: &[u8],
        bpt_total_supply: u64,
    ) -> Result<(u64, u128)> {
        require!(
            actual_balances.len() == amounts_in.len(),
            ErrorCode::InvalidArrayLength
        );

        let mut ratio_min: Option<u128> = None;

        for i in 0..actual_balances.len() {
            if actual_balances[i] > 0 {
                let current_ratio = FixedPoint::div_down(
                    amounts_in[i] as u128,
                    actual_balances[i] as u128,
                )?;

                ratio_min = match ratio_min {
                    None => Some(current_ratio),
                    Some(min) => Some(std::cmp::min(min, current_ratio)),
                };
            }
        }

        // `None` means the loop above never saw a token with
        // `actual_balance > 0` — the pool holds no real liquidity, so there is
        // no ratio to price this deposit against. That is pool state, not a
        // bad argument, and it used to report the blanket `InvalidAmounts`,
        // which points an operator at their own input instead of at the pool.
        let ratio = ratio_min.ok_or(ErrorCode::PoolNotSeeded)?;

        let bpt_amount_u128 = (bpt_total_supply as u128)
            .checked_mul(ratio)
            .ok_or(ErrorCode::MathOverflow)? / ONE;
        let bpt_amount = bpt_amount_u128.try_into().map_err(|_| ErrorCode::MathOverflow)?;

        Ok((bpt_amount, ratio))
    }

    /// Proportional-exit: given a BPT amount to burn, compute each
    /// token's payout via `actual_balance_i * (bpt / total_supply)`.
    ///
    /// Rounded down — the pool always retains at least the rounding-loss
    /// micro-balance, which protects LPs against rounding griefing.
    ///
    /// # Parameters
    /// * `actual_balances`  — pre-burn pool balances.
    /// * `bpt_amount`       — caller-supplied BPT to burn.
    /// * `bpt_total_supply` — pre-burn BPT supply (must be > 0).
    /// * `_decimals`        — kept for ABI symmetry.
    ///
    /// # Returns
    /// `Vec<u64>` of token payouts (raw units).
    ///
    /// # Errors
    /// `InvalidBptAmount`, `MathOverflow`.
    ///
    /// # Used by
    /// `instructions::remove_liquidity`.
    pub fn calc_tokens_out_given_bpt_in(
        actual_balances: &[u64],
        bpt_amount: u64,
        bpt_total_supply: u64,
        _decimals: &[u8],
    ) -> Result<Vec<u64>> {
        require!(bpt_total_supply > 0, ErrorCode::InvalidBptAmount);

        let mut amounts_out = Vec::with_capacity(actual_balances.len());

        let ratio = FixedPoint::div_down(
            bpt_amount as u128,
            bpt_total_supply as u128,
        )?;

        for i in 0..actual_balances.len() {
            let amount_calc = FixedPoint::mul_down(actual_balances[i] as u128, ratio)?;
            let amount: u64 = amount_calc.try_into().map_err(|_| ErrorCode::MathOverflow)?;
            amounts_out.push(amount);
        }

        Ok(amounts_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_out_given_in_same_decimals() {
        let virtual_balance_in = 1_000_000_000;
        let virtual_balance_out = 1_000_000_000;
        let amount_in = 100_000_000;
        let actual_balance_out = 1_000_000_000;
        let weight_in = 5000u64;
        let weight_out = 5000u64;

        let result = CofferMath::calc_out_given_in(
            virtual_balance_in, weight_in, virtual_balance_out, weight_out,
            amount_in, actual_balance_out, 9, 9,
        );

        assert!(result.is_ok());
        let amount_out = result.unwrap();
        assert!(amount_out > 0);
        assert!(amount_out < actual_balance_out);
    }

    #[test]
    fn test_calc_out_given_in_different_decimals() {
        let virtual_balance_in = 1_000_000;
        let virtual_balance_out = 1_000_000_000;
        let amount_in = 100_000;
        let actual_balance_out = 1_000_000_000;
        let weight_in = 5000u64;
        let weight_out = 5000u64;

        let result = CofferMath::calc_out_given_in(
            virtual_balance_in, weight_in, virtual_balance_out, weight_out,
            amount_in, actual_balance_out, 6, 9,
        );

        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[test]
    fn test_calc_out_given_in_asymmetric_weights() {
        let virtual_balance_in = 1_000_000_000;
        let virtual_balance_out = 1_000_000_000;
        let amount_in = 100_000_000;
        let actual_balance_out = 1_000_000_000;
        let weight_in = 8000u64;
        let weight_out = 2000u64;

        let result = CofferMath::calc_out_given_in(
            virtual_balance_in, weight_in, virtual_balance_out, weight_out,
            amount_in, actual_balance_out, 9, 9,
        );

        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }
}

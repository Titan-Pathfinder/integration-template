// PORTED VERBATIM from the Coffer contract's `src/math/fixed_point.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! 18-decimal fixed-point math primitives.
//!
//! `ONE = 1e18` is the implicit scale for every operand. Operations that
//! could lose precision come in `_down` / `_up` flavours — pick the
//! variant whose rounding direction is safe for the caller (e.g. user
//! receives tokens → round down; pool keeps tokens → round up).

use crate::coffer::constants::*;
use crate::coffer::errors::ErrorCode;
use crate::coffer::prelude::*;

/// Empty marker struct — every method is a `pub fn` impl block, so the
/// type just namespaces the math.
pub struct FixedPoint;

impl FixedPoint {
    /// `(a * b) / ONE`, rounded down.
    ///
    /// # Parameters
    /// * `a`, `b` — 18-decimal fixed-point values.
    ///
    /// # Returns
    /// 18-decimal fixed-point product (truncated).
    ///
    /// # Errors
    /// `MathOverflow` if `a * b` overflows `u128`.
    ///
    /// # Used by
    /// `CofferMath::calc_out_given_in`,
    /// `CofferMath::calc_tokens_out_given_bpt_in`.
    pub fn mul_down(a: u128, b: u128) -> Result<u128> {
        let product = a.checked_mul(b).ok_or(ErrorCode::MathOverflow)?;
        Ok(product / ONE)
    }

    /// `floor((a * ONE) / b)`. Standard fixed-point division, rounded
    /// down.
    ///
    /// # Parameters
    /// * `a` — numerator.
    /// * `b` — denominator (must be ≠ 0).
    ///
    /// # Errors
    /// `DivisionByZero`, `MathOverflow`.
    ///
    /// # Used by
    /// `CofferMath::calc_out_given_in` (exponent ratio),
    /// `CofferMath::calc_bpt_out_given_exact_tokens_in`,
    /// `CofferMath::calc_tokens_out_given_bpt_in`.
    pub fn div_down(a: u128, b: u128) -> Result<u128> {
        if b == 0 {
            return Err(ErrorCode::DivisionByZero.into());
        }
        let numerator = a.checked_mul(ONE).ok_or(ErrorCode::MathOverflow)?;
        Ok(numerator / b)
    }

    /// `ceil((a * ONE) / b)`. Counterpart of `div_down`.
    ///
    /// # Used by
    /// `CofferMath::calc_out_given_in` (base ratio — round up so the user
    /// receives at most a fair amount).
    pub fn div_up(a: u128, b: u128) -> Result<u128> {
        if b == 0 {
            return Err(ErrorCode::DivisionByZero.into());
        }
        let numerator = a.checked_mul(ONE).ok_or(ErrorCode::MathOverflow)?;
        if numerator == 0 {
            return Ok(0);
        }
        let result = (numerator - 1) / b + 1;
        Ok(result)
    }

    /// `1 - x` in 18-decimal fixed-point.
    ///
    /// # Parameters
    /// * `x` — must be ≤ `ONE`.
    ///
    /// # Errors
    /// `MathOverflow` if `x > ONE`.
    ///
    /// # Used by
    /// `CofferMath::calc_out_given_in`.
    pub fn complement(x: u128) -> Result<u128> {
        if x > ONE {
            return Err(ErrorCode::MathOverflow.into());
        }
        Ok(ONE - x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_down() {
        let a = 2 * ONE;
        let b = 3 * ONE;
        let result = FixedPoint::mul_down(a, b).unwrap();
        assert_eq!(result, 6 * ONE);
    }

    #[test]
    fn test_div_down() {
        let a = 6 * ONE;
        let b = 2 * ONE;
        let result = FixedPoint::div_down(a, b).unwrap();
        assert_eq!(result, 3 * ONE);
    }

    #[test]
    fn test_div_up_rounds_up() {
        // (1 * ONE) / 3 with round-up = ceil(ONE/3) = ONE/3 + 1
        let result = FixedPoint::div_up(1, 3).unwrap();
        assert!(result > ONE / 3);
    }

    #[test]
    fn test_complement() {
        let x = ONE / 4;
        let result = FixedPoint::complement(x).unwrap();
        assert_eq!(result, 3 * ONE / 4);
    }
}

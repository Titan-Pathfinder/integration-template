// PORTED VERBATIM from the Coffer contract's `src/math/log_exp_math.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! `LogExpMath` — fixed-point 18-decimal `ln`, `exp`, and `pow`.
//!
//! Port of Balancer V2's audited `LogExpMath.sol` to Rust/`u128`,
//! adapted for Solana. Uses 256-bit intermediate precision (via
//! `mul_div_down`) to avoid overflow during multiply-divide. Maintains
//! ~18 significant digits of precision and handles exponent arguments
//! up to ~46 (sufficient for any realistic 18-decimal-scaled token
//! balance).
//!
//! # Reference
//! https://github.com/balancer/balancer-v2-monorepo/blob/master/pkg/solidity-utils/contracts/math/LogExpMath.sol
//!
//! # Differences from the Solidity original
//! - `u128` / `i128` instead of `int256`.
//! - 18-decimal precision throughout (the Solidity uses 20-decimal
//!   intermediates, which would overflow in `u128`).
//! - 10-step decomposition table baked from Balancer's audited
//!   20-decimal constants divided by 100.

use crate::coffer::errors::ErrorCode;
use crate::coffer::prelude::*;

/// 1e18 — local copy keeps this module self-contained / pasteable.
const ONE: u128 = 1_000_000_000_000_000_000;

/// `e^46 ≈ 9.5e19 * ONE ≈ 9.5e37` — the largest exponent that `exp()`
/// can return without overflowing `u128`.
const MAX_NATURAL_EXPONENT: i128 = 46_000_000_000_000_000_000;

/// `e^(-41) ≈ 1.5e-18` — the most negative exponent that still produces
/// a non-zero result in 18-decimal precision.
const MIN_NATURAL_EXPONENT: i128 = -41_000_000_000_000_000_000;

// Decomposition constants: a_i = e^(x_i) * ONE.
// Derived from Balancer's audited 20-decimal table / 100.
const X0: u128 =  32_000_000_000_000_000_000;
const A0: u128 =  78_962_960_182_680_695_161_000_000_000_000;

const X1: u128 =  16_000_000_000_000_000_000;
const A1: u128 =  8_886_110_520_507_872_636_760_000;

const X2: u128 =  8_000_000_000_000_000_000;
const A2: u128 =  2_980_957_987_041_728_274_740;

const X3: u128 =  4_000_000_000_000_000_000;
const A3: u128 =  54_598_150_033_144_239_078;

const X4: u128 =  2_000_000_000_000_000_000;
const A4: u128 =  7_389_056_098_930_650_227;

const X5: u128 =  1_000_000_000_000_000_000;
const A5: u128 =  2_718_281_828_459_045_235;

const X6: u128 =  500_000_000_000_000_000;
const A6: u128 =  1_648_721_270_700_128_146;

const X7: u128 =  250_000_000_000_000_000;
const A7: u128 =  1_284_025_416_687_741_484;

const X8: u128 =  125_000_000_000_000_000;
const A8: u128 =  1_133_148_453_066_826_316;

const X9: u128 =  62_500_000_000_000_000;
const A9: u128 =  1_064_494_458_917_859_429;

const STEPS: usize = 10;
const X_STEPS: [u128; STEPS] = [X0, X1, X2, X3, X4, X5, X6, X7, X8, X9];
const A_STEPS: [u128; STEPS] = [A0, A1, A2, A3, A4, A5, A6, A7, A8, A9];

/// Number of Taylor terms used in `exp()`'s post-decomposition tail.
const EXP_TAYLOR_TERMS: usize = 12;

/// Number of Taylor terms used in `ln()`'s tail.
const LN_TAYLOR_TERMS: usize = 6;

/// Namespace for log/exp/pow math.
pub struct LogExpMath;

impl LogExpMath {
    /// Compute `x^y` in 18-decimal fixed-point via `exp(y * ln(x))`.
    ///
    /// # Parameters
    /// * `x` — base, > 0, in 18-decimal fixed-point.
    /// * `y` — exponent in `[0, ONE]` (e.g. `0.5 = 5e17`).
    ///
    /// # Returns
    /// `x^y` in 18-decimal fixed-point. Special cases:
    /// `pow(x, 0) = ONE`, `pow(0, y) = 0`, `pow(ONE, y) = ONE`.
    ///
    /// # Errors
    /// `MathOverflow` if `y * ln(x)` falls outside `[MIN_NATURAL_EXPONENT,
    /// MAX_NATURAL_EXPONENT]`.
    ///
    /// # Used by
    /// `WeightedMath::calculate_invariant` (every pool init / first
    /// deposit), `CofferMath::calc_out_given_in` (every swap).
    pub fn pow(x: u128, y: u128) -> Result<u128> {
        if y == 0 {
            return Ok(ONE);
        }
        if x == 0 {
            return Ok(0);
        }
        if x == ONE {
            return Ok(ONE);
        }

        let ln_x = Self::ln(x)?;

        let y_i = y as i128;
        let one_i = ONE as i128;

        let int_part = ln_x / one_i;
        let frac_part = ln_x % one_i;

        let logx_times_y = int_part
            .checked_mul(y_i)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_add(
                frac_part
                    .checked_mul(y_i)
                    .ok_or(ErrorCode::MathOverflow)?
                    / one_i,
            )
            .ok_or(ErrorCode::MathOverflow)?;

        require!(
            logx_times_y >= MIN_NATURAL_EXPONENT && logx_times_y <= MAX_NATURAL_EXPONENT,
            ErrorCode::MathOverflow
        );

        Self::exp(logx_times_y)
    }

    /// Natural log in 18-decimal fixed-point.
    ///
    /// # Parameters
    /// * `a` — must be > 0.
    ///
    /// # Returns
    /// `i128` (signed: negative for `a < ONE`) in 18-decimal.
    ///
    /// # Errors
    /// `DivisionByZero` if `a == 0`.
    ///
    /// # Used by
    /// `Self::pow` only.
    pub fn ln(a: u128) -> Result<i128> {
        require!(a > 0, ErrorCode::DivisionByZero);

        if a < ONE {
            let a_inv = mul_div_down(ONE, ONE, a)?;
            return Ok(-Self::ln_positive(a_inv)?);
        }

        Self::ln_positive(a)
    }

    /// `ln` for `a >= ONE`. Always returns ≥ 0.
    /// Internal — public callers use `Self::ln`.
    fn ln_positive(mut a: u128) -> Result<i128> {
        let mut sum: i128 = 0;

        for i in 0..STEPS {
            if a >= A_STEPS[i] {
                a = mul_div_down(a, ONE, A_STEPS[i])?;
                sum += X_STEPS[i] as i128;
            }
        }

        let a_minus_one = a.checked_sub(ONE).ok_or(ErrorCode::MathUnderflow)?;
        let a_plus_one = a.checked_add(ONE).ok_or(ErrorCode::MathOverflow)?;
        let z = mul_div_down(a_minus_one, ONE, a_plus_one)? as i128;

        let one_i = ONE as i128;
        let z_sq = z.checked_mul(z).ok_or(ErrorCode::MathOverflow)? / one_i;

        let mut num = z;
        let mut series_sum = num;

        for k in 1..LN_TAYLOR_TERMS {
            num = num.checked_mul(z_sq).ok_or(ErrorCode::MathOverflow)? / one_i;
            let divisor = (2 * k + 1) as i128;
            series_sum = series_sum
                .checked_add(num / divisor)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        series_sum = series_sum
            .checked_mul(2)
            .ok_or(ErrorCode::MathOverflow)?;

        Ok(sum + series_sum)
    }

    /// Natural exponential `e^x` in 18-decimal fixed-point.
    ///
    /// # Parameters
    /// * `x` — signed `i128` in 18-decimal.
    ///
    /// # Returns
    /// `u128` (unsigned because `e^x > 0`) in 18-decimal.
    ///
    /// # Errors
    /// `MathOverflow` outside `[MIN_NATURAL_EXPONENT,
    /// MAX_NATURAL_EXPONENT]`.
    ///
    /// # Used by
    /// `Self::pow` only.
    pub fn exp(x: i128) -> Result<u128> {
        require!(
            x >= MIN_NATURAL_EXPONENT && x <= MAX_NATURAL_EXPONENT,
            ErrorCode::MathOverflow
        );

        if x < 0 {
            let pos = Self::exp(-x)?;
            return mul_div_down(ONE, ONE, pos);
        }

        let mut x = x as u128;
        let mut product: u128 = ONE;

        for i in 0..STEPS {
            if x >= X_STEPS[i] {
                x -= X_STEPS[i];
                product = mul_div_down(product, A_STEPS[i], ONE)?;
            }
        }

        let mut series_sum: u128 = ONE;
        let mut term: u128 = x;

        series_sum = series_sum.checked_add(term).ok_or(ErrorCode::MathOverflow)?;

        for n in 2..=EXP_TAYLOR_TERMS {
            term = mul_div_down(term, x, ONE)? / (n as u128);
            series_sum = series_sum
                .checked_add(term)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        mul_div_down(product, series_sum, ONE)
    }
}

/// `(a * b) / denom` rounded down, with 256-bit intermediate to
/// gracefully handle overflowing products.
///
/// # Parameters
/// * `a`, `b`, `denom` — `u128` operands; `denom` must be > 0.
///
/// # Returns
/// `floor((a * b) / denom)` if it fits in `u128`.
///
/// # Errors
/// * `DivisionByZero` if `denom == 0`.
/// * `MathOverflow` only when the true quotient exceeds `u128::MAX`.
///
/// # Used by
/// `LogExpMath::ln`, `LogExpMath::exp`, `LogExpMath::pow`,
/// `WeightedMath::calculate_invariant`.
pub fn mul_div_down(a: u128, b: u128, denom: u128) -> Result<u128> {
    if denom == 0 {
        return Err(ErrorCode::DivisionByZero.into());
    }
    if a == 0 || b == 0 {
        return Ok(0);
    }

    if let Some(product) = a.checked_mul(b) {
        return Ok(product / denom);
    }

    let (hi, lo) = u128_mul_wide(a, b);
    div_256_by_128(hi, lo, denom)
}

/// 128×128 → 256-bit multiplication, returned as `(high, low)`. Internal
/// helper for `mul_div_down`'s slow path.
fn u128_mul_wide(a: u128, b: u128) -> (u128, u128) {
    let mask: u128 = u64::MAX as u128;

    let a_lo = a & mask;
    let a_hi = a >> 64;
    let b_lo = b & mask;
    let b_hi = b >> 64;

    let p0 = a_lo * b_lo;
    let p1 = a_hi * b_lo;
    let p2 = a_lo * b_hi;
    let p3 = a_hi * b_hi;

    let mid = (p0 >> 64)
        .wrapping_add(p1 & mask)
        .wrapping_add(p2 & mask);

    let lo = (p0 & mask) | ((mid & mask) << 64);
    let hi = p3
        .wrapping_add(p1 >> 64)
        .wrapping_add(p2 >> 64)
        .wrapping_add(mid >> 64);

    (hi, lo)
}

/// 256-bit / 128-bit division. Internal helper for `mul_div_down`'s slow
/// path. Requires `hi < denom` (which is guaranteed when the quotient
/// fits in `u128`).
fn div_256_by_128(hi: u128, lo: u128, denom: u128) -> Result<u128> {
    if denom == 0 {
        return Err(ErrorCode::DivisionByZero.into());
    }
    if hi >= denom {
        return Err(ErrorCode::MathOverflow.into());
    }
    if hi == 0 {
        return Ok(lo / denom);
    }

    let mask: u128 = u64::MAX as u128;

    if denom <= mask {
        let d1 = (hi << 64) | (lo >> 64);
        let q1 = d1 / denom;
        let r1 = d1 % denom;
        let d2 = (r1 << 64) | (lo & mask);
        let q2 = d2 / denom;
        return Ok((q1 << 64) | q2);
    }

    let mut rem = hi;
    let mut quot: u128 = 0;
    for i in (0..128u32).rev() {
        let bit = (lo >> i) & 1;
        let overflow = rem >> 127;
        rem = (rem << 1) | bit;
        if overflow != 0 || rem >= denom {
            rem -= denom;
            quot |= 1u128 << i;
        }
    }
    Ok(quot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_div_basic() {
        assert_eq!(mul_div_down(10, 20, 5).unwrap(), 40);
        assert_eq!(mul_div_down(ONE, ONE, ONE).unwrap(), ONE);
        assert_eq!(mul_div_down(0, ONE, ONE).unwrap(), 0);
    }

    #[test]
    fn test_mul_div_overflow_path() {
        let a: u128 = u128::MAX / 2;
        let b: u128 = 4;
        let denom: u128 = 2;
        let result = mul_div_down(a, b, denom).unwrap();
        assert_eq!(result, u128::MAX / 2 * 2);
    }

    #[test]
    fn test_mul_div_large_denom() {
        let a = ONE * 100;
        let b = ONE * 200;
        let denom = ONE;
        let result = mul_div_down(a, b, denom).unwrap();
        assert_eq!(result, ONE * 20_000);
    }

    #[test]
    fn test_exp_zero() {
        assert_eq!(LogExpMath::exp(0).unwrap(), ONE);
    }

    #[test]
    fn test_exp_one() {
        let result = LogExpMath::exp(ONE as i128).unwrap();
        let e_times_one = 2_718_281_828_459_045_235u128;
        let diff = if result > e_times_one {
            result - e_times_one
        } else {
            e_times_one - result
        };
        assert!(diff < 1_000_000, "exp(1) = {} vs expected {}", result, e_times_one);
    }

    #[test]
    fn test_exp_negative() {
        let result = LogExpMath::exp(-(ONE as i128)).unwrap();
        let expected = 367_879_441_171_442_321u128;
        let diff = if result > expected { result - expected } else { expected - result };
        assert!(diff < 1_000_000_000, "exp(-1) = {} vs expected {}", result, expected);
    }

    #[test]
    fn test_exp_large() {
        let result = LogExpMath::exp(44 * ONE as i128).unwrap();
        assert!(result > ONE);
        let real_value = result / ONE;
        assert!(real_value > 1_000_000_000_000_000_000, "exp(44) real ≈ 1.29e19, got {}", real_value);
        assert!(real_value < 20_000_000_000_000_000_000u128, "too large");
    }

    #[test]
    fn test_ln_one() {
        assert_eq!(LogExpMath::ln(ONE).unwrap(), 0);
    }

    #[test]
    fn test_ln_e() {
        let e_18 = 2_718_281_828_459_045_235u128;
        let result = LogExpMath::ln(e_18).unwrap();
        let diff = (result - ONE as i128).unsigned_abs();
        assert!(diff < 1_000_000_000, "ln(e) = {} vs ONE = {}", result, ONE);
    }

    #[test]
    fn test_ln_less_than_one() {
        let half = ONE / 2;
        let result = LogExpMath::ln(half).unwrap();
        assert!(result < 0);
        let expected = -693_147_180_559_945_309i128;
        let diff = (result - expected).unsigned_abs();
        assert!(diff < 1_000_000_000, "ln(0.5) = {} vs expected {}", result, expected);
    }

    #[test]
    fn test_ln_large() {
        let val = 10_000_000_000_000_000_000_000_000_000u128;
        let result = LogExpMath::ln(val).unwrap();
        let expected = 23_025_850_929_940_456_840i128;
        let diff = (result - expected).unsigned_abs();
        assert!(diff < 1_000_000_000_000, "ln(1e28) = {} vs expected {}", result, expected);
    }

    #[test]
    fn test_pow_identity() {
        let result = LogExpMath::pow(ONE * 5, ONE).unwrap();
        let expected = ONE * 5;
        let diff = if result > expected { result - expected } else { expected - result };
        assert!(diff < 1_000, "x^1 = {} vs expected {}, diff = {}", result, expected, diff);
    }

    #[test]
    fn test_pow_zero_exponent() {
        assert_eq!(LogExpMath::pow(ONE * 5, 0).unwrap(), ONE);
    }

    #[test]
    fn test_pow_zero_base() {
        assert_eq!(LogExpMath::pow(0, ONE / 2).unwrap(), 0);
    }

    #[test]
    fn test_pow_sqrt() {
        let four = ONE * 4;
        let half = ONE / 2;
        let result = LogExpMath::pow(four, half).unwrap();
        let expected = ONE * 2;
        let diff = if result > expected { result - expected } else { expected - result };
        assert!(diff < 1_000_000_000, "sqrt(4) = {} vs expected {}", result, expected);
    }

    #[test]
    fn test_pow_cube_root() {
        let twenty_seven = ONE * 27;
        let third = ONE / 3;
        let result = LogExpMath::pow(twenty_seven, third).unwrap();
        let expected = ONE * 3;
        let diff = if result > expected { result - expected } else { expected - result };
        assert!(
            diff < 10_000_000_000,
            "cbrt(27) = {} vs expected {}, diff = {}",
            result, expected, diff
        );
    }

    #[test]
    fn test_pow_large_balance_50_50() {
        let balance = ONE;
        let weight = ONE / 2;
        let result = LogExpMath::pow(balance, weight).unwrap();
        assert_eq!(result, ONE);
    }

    #[test]
    fn test_pow_large_balance_real() {
        let balance = 1_000_000_000_000_000_000_000u128;
        let weight = ONE / 2;
        let result = LogExpMath::pow(balance, weight).unwrap();
        let expected = 31_622_776_601_683_793_319u128;
        let diff = if result > expected { result - expected } else { expected - result };
        let tolerance = expected / 1_000_000;
        assert!(
            diff < tolerance,
            "pow(1e21, 0.5) = {} vs expected {}, diff = {}",
            result, expected, diff
        );
    }

    #[test]
    fn test_pow_fractional_base() {
        let base = ONE / 1000;
        let weight = ONE / 2;
        let result = LogExpMath::pow(base, weight).unwrap();
        let expected = 31_622_776_601_683_793u128;
        let diff = if result > expected { result - expected } else { expected - result };
        let tolerance = expected / 100_000;
        assert!(
            diff < tolerance,
            "pow(0.001, 0.5) = {} vs expected {}, diff = {}",
            result, expected, diff
        );
    }

    #[test]
    fn test_invariant_correctness_50_50() {
        let balance = 1_000_000_000_000_000_000_000u128;
        let weight = ONE / 2;

        let powered = LogExpMath::pow(balance, weight).unwrap();
        let invariant = mul_div_down(powered, powered, ONE).unwrap();

        let expected = balance;
        let diff = if invariant > expected {
            invariant - expected
        } else {
            expected - invariant
        };
        let tolerance = expected / 1_000_000;
        assert!(
            diff < tolerance,
            "invariant = {} vs expected {}, diff = {}",
            invariant, expected, diff
        );
    }

    #[test]
    fn test_invariant_preserved_after_swap() {
        let w = ONE / 2;
        let b_a_before = 1_000_000_000_000_000_000_000u128;
        let b_b_before = 1_000_000_000_000_000_000_000u128;

        let p_a = LogExpMath::pow(b_a_before, w).unwrap();
        let p_b = LogExpMath::pow(b_b_before, w).unwrap();
        let inv_before = mul_div_down(p_a, p_b, ONE).unwrap();

        let b_a_after = 1_100_000_000_000_000_000_000u128;
        let b_b_after = 909_090_909_090_909_090_909u128;

        let p_a2 = LogExpMath::pow(b_a_after, w).unwrap();
        let p_b2 = LogExpMath::pow(b_b_after, w).unwrap();
        let inv_after = mul_div_down(p_a2, p_b2, ONE).unwrap();

        let diff = if inv_after > inv_before {
            inv_after - inv_before
        } else {
            inv_before - inv_after
        };
        let tolerance = inv_before / 10_000;
        assert!(
            diff < tolerance,
            "invariant drift: before = {}, after = {}, diff = {}",
            inv_before, inv_after, diff
        );
    }
}

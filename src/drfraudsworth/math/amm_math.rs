// Source: programs/amm/src/helpers/math.rs -- MUST stay in sync
//
// Exact copy of pure swap math functions from the on-chain AMM program.
// These functions operate on primitives only (no Solana deps).

/// Calculate effective input after LP fee deduction.
///
/// Formula: `amount_in * (10_000 - fee_bps) / 10_000`
///
/// # Returns
/// * `Some(effective_input)` as u128 for downstream multiplication headroom
/// * `None` if fee_bps > 10_000 (underflow) or arithmetic overflow
pub fn calculate_effective_input(amount_in: u64, fee_bps: u16) -> Option<u128> {
    let amount = amount_in as u128;
    let fee_factor = 10_000u128.checked_sub(fee_bps as u128)?;
    amount.checked_mul(fee_factor)?.checked_div(10_000)
}

/// Calculate swap output using constant-product formula.
///
/// Formula: `reserve_out * effective_input / (reserve_in + effective_input)`
///
/// Integer division truncates (rounds down) -- the protocol keeps dust.
pub fn calculate_swap_output(
    reserve_in: u64,
    reserve_out: u64,
    effective_input: u128,
) -> Option<u64> {
    let r_in = reserve_in as u128;
    let r_out = reserve_out as u128;

    let numerator = r_out.checked_mul(effective_input)?;
    let denominator = r_in.checked_add(effective_input)?;

    if denominator == 0 {
        return None;
    }

    let output = numerator.checked_div(denominator)?;

    u64::try_from(output).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_100bps_on_1000() {
        assert_eq!(calculate_effective_input(1000, 100), Some(990));
    }

    #[test]
    fn fee_zero_bps() {
        assert_eq!(calculate_effective_input(1000, 0), Some(1000));
    }

    #[test]
    fn fee_over_10000_bps() {
        assert_eq!(calculate_effective_input(1000, 10001), None);
    }

    #[test]
    fn fee_on_zero_amount() {
        assert_eq!(calculate_effective_input(0, 100), Some(0));
    }

    #[test]
    fn swap_equal_reserves_1m() {
        assert_eq!(calculate_swap_output(1_000_000, 1_000_000, 1000), Some(999));
    }

    #[test]
    fn swap_zero_effective_input() {
        assert_eq!(calculate_swap_output(1_000_000, 1_000_000, 0), Some(0));
    }

    #[test]
    fn swap_zero_reserve_in_zero_effective() {
        assert_eq!(calculate_swap_output(0, 1_000_000, 0), None);
    }
}

// Source: programs/tax-program/src/helpers/tax_math.rs -- MUST stay in sync
//
// Exact copy of pure tax calculation functions from the on-chain Tax Program.
// These functions operate on primitives only (no Solana deps).

/// Calculate tax amount from a lamport value and tax rate in basis points.
///
/// Formula: `amount_lamports * tax_bps / 10_000`
pub fn calculate_tax(amount_lamports: u64, tax_bps: u16) -> Option<u64> {
    if tax_bps > 10_000 {
        return None;
    }

    let amount = amount_lamports as u128;
    let bps = tax_bps as u128;

    let tax = amount
        .checked_mul(bps)?
        .checked_div(10_000)?;

    u64::try_from(tax).ok()
}

/// Split total tax into (staking, carnage, treasury) portions.
///
/// Distribution (71/24/5 split):
/// - Staking: 71% (floor)
/// - Carnage: 24% (floor)
/// - Treasury: remainder (absorbs rounding dust)
pub fn split_distribution(total_tax: u64) -> Option<(u64, u64, u64)> {
    const STAKING_BPS: u128 = 7_100;
    const CARNAGE_BPS: u128 = 2_400;
    const BPS_DENOM: u128 = 10_000;

    if total_tax < 4 {
        return Some((total_tax, 0, 0));
    }

    let total = total_tax as u128;

    let staking_u128 = total.checked_mul(STAKING_BPS)?.checked_div(BPS_DENOM)?;
    let staking = u64::try_from(staking_u128).ok()?;

    let carnage_u128 = total.checked_mul(CARNAGE_BPS)?.checked_div(BPS_DENOM)?;
    let carnage = u64::try_from(carnage_u128).ok()?;

    let treasury = total_tax
        .checked_sub(staking)?
        .checked_sub(carnage)?;

    Some((staking, carnage, treasury))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tax_4pct_on_1_sol() {
        assert_eq!(calculate_tax(1_000_000_000, 400), Some(40_000_000));
    }

    #[test]
    fn tax_zero_input() {
        assert_eq!(calculate_tax(0, 400), Some(0));
    }

    #[test]
    fn tax_invalid_bps() {
        assert_eq!(calculate_tax(1_000_000_000, 10001), None);
    }

    #[test]
    fn split_100_lamports() {
        assert_eq!(split_distribution(100), Some((71, 24, 5)));
    }

    #[test]
    fn split_micro_tax_3_lamports() {
        assert_eq!(split_distribution(3), Some((3, 0, 0)));
    }

    #[test]
    fn split_invariant_sum_equals_total() {
        for total in [0, 1, 2, 3, 4, 5, 10, 99, 100, 101, 1000, 10000, 1_000_000] {
            let (staking, carnage, treasury) = split_distribution(total).unwrap();
            assert_eq!(staking + carnage + treasury, total);
        }
    }
}

// Source: programs/conversion-vault/src/instructions/convert.rs -- MUST stay in sync
//
// SDK mirror of the on-chain vault conversion math.

use solana_sdk::pubkey::Pubkey;
use crate::drfraudsworth::accounts::addresses::{CRIME_MINT, FRAUD_MINT, PROFIT_MINT};
use crate::drfraudsworth::constants::CONVERSION_RATE;

/// Compute the output amount for a vault conversion.
///
/// # Conversion rules
/// - CRIME/FRAUD -> PROFIT: divide by CONVERSION_RATE (100)
/// - PROFIT -> CRIME/FRAUD: multiply by CONVERSION_RATE (100)
/// - CRIME <-> FRAUD: Not supported on-chain.
/// - Same mint / zero amount: Not supported.
pub fn compute_vault_output(
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount_in: u64,
) -> Option<u64> {
    if amount_in == 0 {
        return None;
    }
    if input_mint == output_mint {
        return None;
    }

    let is_input_crime_or_fraud =
        *input_mint == CRIME_MINT || *input_mint == FRAUD_MINT;
    let is_output_profit = *output_mint == PROFIT_MINT;
    let is_input_profit = *input_mint == PROFIT_MINT;
    let is_output_crime_or_fraud =
        *output_mint == CRIME_MINT || *output_mint == FRAUD_MINT;

    if is_input_crime_or_fraud && is_output_profit {
        let out = amount_in / CONVERSION_RATE;
        if out == 0 {
            return None;
        }
        Some(out)
    } else if is_input_profit && is_output_crime_or_fraud {
        amount_in.checked_mul(CONVERSION_RATE)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crime_to_profit() {
        assert_eq!(compute_vault_output(&CRIME_MINT, &PROFIT_MINT, 10_000), Some(100));
    }

    #[test]
    fn profit_to_crime() {
        assert_eq!(compute_vault_output(&PROFIT_MINT, &CRIME_MINT, 100), Some(10_000));
    }

    #[test]
    fn crime_to_fraud_not_supported() {
        assert_eq!(compute_vault_output(&CRIME_MINT, &FRAUD_MINT, 1000), None);
    }

    #[test]
    fn zero_amount() {
        assert_eq!(compute_vault_output(&CRIME_MINT, &PROFIT_MINT, 0), None);
    }

    #[test]
    fn dust_too_small() {
        assert_eq!(compute_vault_output(&CRIME_MINT, &PROFIT_MINT, 99), None);
    }
}

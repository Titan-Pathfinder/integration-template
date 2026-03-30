// Hardcoded TokenInfo for Dr. Fraudsworth protocol mints.
//
// Our tokens use Transfer Hook (whitelist enforcement), NOT the Transfer Fee
// extension. So transfer_fee = None for all T22 mints. Titan handles
// Transfer Fee extension externally if present.

use crate::trading_venue::token_info::TokenInfo;
use solana_sdk::pubkey::Pubkey;

use crate::drfraudsworth::accounts::addresses::{CRIME_MINT, FRAUD_MINT, NATIVE_MINT, PROFIT_MINT};
use crate::drfraudsworth::constants::{SOL_DECIMALS, TOKEN_DECIMALS};

/// Build a TokenInfo for a known protocol mint.
///
/// Returns None for unknown mints.
pub fn token_info_for_mint(mint: &Pubkey) -> Option<TokenInfo> {
    if *mint == NATIVE_MINT {
        Some(TokenInfo {
            pubkey: NATIVE_MINT,
            decimals: SOL_DECIMALS as i32,
            is_token_2022: false,
            transfer_fee: None,
            maximum_fee: None,
        })
    } else if *mint == CRIME_MINT {
        Some(TokenInfo {
            pubkey: CRIME_MINT,
            decimals: TOKEN_DECIMALS as i32,
            is_token_2022: true,
            transfer_fee: None,
            maximum_fee: None,
        })
    } else if *mint == FRAUD_MINT {
        Some(TokenInfo {
            pubkey: FRAUD_MINT,
            decimals: TOKEN_DECIMALS as i32,
            is_token_2022: true,
            transfer_fee: None,
            maximum_fee: None,
        })
    } else if *mint == PROFIT_MINT {
        Some(TokenInfo {
            pubkey: PROFIT_MINT,
            decimals: TOKEN_DECIMALS as i32,
            is_token_2022: true,
            transfer_fee: None,
            maximum_fee: None,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sol_is_not_token_2022() {
        let info = token_info_for_mint(&NATIVE_MINT).unwrap();
        assert!(!info.is_token_2022);
        assert_eq!(info.decimals, 9);
        assert!(info.transfer_fee.is_none());
    }

    #[test]
    fn crime_is_token_2022_no_transfer_fee() {
        let info = token_info_for_mint(&CRIME_MINT).unwrap();
        assert!(info.is_token_2022);
        assert_eq!(info.decimals, 6);
        assert!(info.transfer_fee.is_none());
    }

    #[test]
    fn fraud_is_token_2022_no_transfer_fee() {
        let info = token_info_for_mint(&FRAUD_MINT).unwrap();
        assert!(info.is_token_2022);
        assert_eq!(info.decimals, 6);
    }

    #[test]
    fn profit_is_token_2022_no_transfer_fee() {
        let info = token_info_for_mint(&PROFIT_MINT).unwrap();
        assert!(info.is_token_2022);
        assert_eq!(info.decimals, 6);
    }

    #[test]
    fn unknown_mint_returns_none() {
        assert!(token_info_for_mint(&Pubkey::new_unique()).is_none());
    }
}

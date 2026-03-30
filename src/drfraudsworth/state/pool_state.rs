// Raw byte parser for AMM PoolState account data.
//
// Byte offsets verified against programs/tax-program/src/helpers/pool_reader.rs.
//
// PoolState byte layout:
//   [0..8]     Anchor discriminator
//   [8]        pool_type (1 byte, enum)
//   [9..41]    mint_a (Pubkey, 32 bytes)
//   [41..73]   mint_b (Pubkey, 32 bytes)
//   [73..105]  vault_a (Pubkey, 32 bytes)
//   [105..137] vault_b (Pubkey, 32 bytes)
//   [137..145] reserve_a (u64, 8 bytes)
//   [145..153] reserve_b (u64, 8 bytes)
//   [153..155] lp_fee_bps (u16, 2 bytes)

use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;

use crate::drfraudsworth::accounts::addresses::NATIVE_MINT;

const MIN_LEN: usize = 155;

#[derive(Debug, Clone, Copy)]
pub struct ParsedPoolState {
    pub mint_a: Pubkey,
    pub reserve_a: u64,
    pub reserve_b: u64,
    pub lp_fee_bps: u16,
}

impl ParsedPoolState {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < MIN_LEN {
            return Err(anyhow!(
                "PoolState data too short: {} bytes (need {})",
                data.len(),
                MIN_LEN
            ));
        }

        let mint_a = Pubkey::try_from(&data[9..41])
            .map_err(|_| anyhow!("Failed to parse mint_a from bytes [9..41]"))?;

        let reserve_a = u64::from_le_bytes(
            data[137..145].try_into()
                .map_err(|_| anyhow!("Failed to parse reserve_a from bytes [137..145]"))?
        );

        let reserve_b = u64::from_le_bytes(
            data[145..153].try_into()
                .map_err(|_| anyhow!("Failed to parse reserve_b from bytes [145..153]"))?
        );

        let lp_fee_bps = u16::from_le_bytes([data[153], data[154]]);

        Ok(Self {
            mint_a,
            reserve_a,
            reserve_b,
            lp_fee_bps,
        })
    }

    /// Returns (sol_reserve, token_reserve) with is_reversed detection.
    pub fn sol_and_token_reserves(&self) -> (u64, u64) {
        if self.mint_a == NATIVE_MINT {
            (self.reserve_a, self.reserve_b)
        } else {
            (self.reserve_b, self.reserve_a)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_pool_state(mint_a: &Pubkey, reserve_a: u64, reserve_b: u64, lp_fee_bps: u16) -> Vec<u8> {
        let mut data = vec![0u8; 224];
        data[8] = 0;
        data[9..41].copy_from_slice(mint_a.as_ref());
        data[137..145].copy_from_slice(&reserve_a.to_le_bytes());
        data[145..153].copy_from_slice(&reserve_b.to_le_bytes());
        data[153..155].copy_from_slice(&lp_fee_bps.to_le_bytes());
        data
    }

    #[test]
    fn parse_normal_order_sol_pool() {
        let data = mock_pool_state(&NATIVE_MINT, 100_000_000, 500_000_000, 100);
        let parsed = ParsedPoolState::from_bytes(&data).unwrap();
        assert_eq!(parsed.mint_a, NATIVE_MINT);
        let (sol, token) = parsed.sol_and_token_reserves();
        assert_eq!(sol, 100_000_000);
        assert_eq!(token, 500_000_000);
    }

    #[test]
    fn parse_reversed_order_pool() {
        let some_mint = Pubkey::new_unique();
        let data = mock_pool_state(&some_mint, 500_000_000, 100_000_000, 100);
        let parsed = ParsedPoolState::from_bytes(&data).unwrap();
        let (sol, token) = parsed.sol_and_token_reserves();
        assert_eq!(sol, 100_000_000);
        assert_eq!(token, 500_000_000);
    }

    #[test]
    fn reject_too_short() {
        assert!(ParsedPoolState::from_bytes(&[0u8; 100]).is_err());
    }
}

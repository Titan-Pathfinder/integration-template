//! Ocelot v3 CPI adapter. Pinned ExactIn data is rejected if routing changes its amount.
use crate::error::TemplateError;
use anchor_lang::{prelude::*, solana_program::instruction::Instruction};
pub const OCELOT_PROGRAM_ID: Pubkey = pubkey!("CpLah6JAC9967YiVQnQJegfCKEjLEcUYmx3PCYgg9xWf");
const MINT: [u8; 8] = [190, 51, 100, 237, 14, 73, 165, 91];
const REDEEM: [u8; 8] = [207, 0, 207, 230, 122, 44, 120, 250];
pub fn swap(
    data: &[u8; 160],
    len: u8,
    quoted_input: u64,
    amount: u64,
    accounts: &[AccountMeta],
) -> Result<Vec<Instruction>> {
    let n = len as usize;
    require!(
        amount > 0 && amount == quoted_input && n <= data.len() && n >= 55,
        TemplateError::InvalidSwapInput
    );
    let mint = data[..8] == MINT;
    require!(
        (mint && n == 148) || (data[..8] == REDEEM && (n == 112 || n == 55)),
        TemplateError::InvalidSwapInput
    );
    let encoded = u64::from_le_bytes(
        data[8..16]
            .try_into()
            .map_err(|_| TemplateError::InvalidSwapInput)?,
    );
    let minimum = u64::from_le_bytes(
        data[n - 24..n - 16]
            .try_into()
            .map_err(|_| TemplateError::InvalidSwapInput)?,
    );
    require!(
        encoded == amount && minimum > 0,
        TemplateError::InvalidSwapInput
    );
    // Ocelot verifies every account binding, full-fill IOC, deadline and finance revision.
    // Whitelisting only these discriminators prevents arbitrary admin/keeper CPIs.
    Ok(vec![Instruction {
        program_id: OCELOT_PROGRAM_ID,
        accounts: accounts.to_vec(),
        data: data[..n].to_vec(),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_guarded_v3_mint_and_redeem() {
        for (disc, len) in [(MINT, 148usize), (REDEEM, 112), (REDEEM, 55)] {
            let mut data = [0; 160];
            data[..8].copy_from_slice(&disc);
            data[8..16].copy_from_slice(&100u64.to_le_bytes());
            data[len - 24..len - 16].copy_from_slice(&1u64.to_le_bytes());
            let ix = swap(&data, len as u8, 100, 100, &[]).unwrap();
            assert_eq!(ix[0].program_id, OCELOT_PROGRAM_ID);
            assert_eq!(ix[0].data, data[..len]);
            assert!(swap(&data, len as u8, 100, 101, &[]).is_err());
            assert!(swap(&data, len as u8, 101, 101, &[]).is_err());
            data[len - 24..len - 16].fill(0);
            assert!(swap(&data, len as u8, 100, 100, &[]).is_err());
            data[len - 24] = 1;
            data[0] ^= 1;
            assert!(swap(&data, len as u8, 100, 100, &[]).is_err());
        }
        for len in [0, 54, 113, 147, 149, 161, 255] {
            assert!(swap(&[0; 160], len, 100, 100, &[]).is_err());
        }
    }
}

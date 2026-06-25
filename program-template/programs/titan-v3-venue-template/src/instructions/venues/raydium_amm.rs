use anchor_lang::{prelude::*, solana_program::instruction::Instruction};

pub const PROGRAM_ID: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

pub fn swap_base_in_v2(amount_in: u64, account_metas: &[AccountMeta]) -> Result<Vec<Instruction>> {
    let mut data = Vec::with_capacity(17);
    data.push(16);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());

    Ok(vec![Instruction {
        program_id: PROGRAM_ID,
        accounts: account_metas.to_vec(),
        data,
    }])
}

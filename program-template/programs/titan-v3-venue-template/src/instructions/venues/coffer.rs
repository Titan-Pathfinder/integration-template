use anchor_lang::{prelude::*, solana_program::instruction::Instruction};

/// Coffer / Cube DEX Coffer program (mainnet).
pub const PROGRAM_ID: Pubkey = pubkey!("8iQtGj9mcUfFUGaiCpPy89swC3s8YTC8FhVZWfgeZhwu");

/// `sha256("global:swap")[..8]`.
const SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];

/// `swap(amount_in, minimum_amount_out, token_in_index, token_out_index)`.
///
/// Accounts are forwarded exactly as the off-chain builder produced them
/// (`CofferVenue::generate_swap_instruction`): pool(w), mint_in, mint_out,
/// TitanPDA's input ATA(w), TitanPDA's output ATA(w), vault_in(w),
/// vault_out(w), TitanPDA (signer via seeds), token_program_in,
/// token_program_out. `minimum_amount_out` is 0. This local router template
/// has no route-level minimum-output check; a production route must provide
/// that protection separately.
pub fn swap(
    token_in_index: u8,
    token_out_index: u8,
    amount_in: u64,
    account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
    let mut data = Vec::with_capacity(26);
    data.extend_from_slice(&SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.push(token_in_index);
    data.push(token_out_index);

    Ok(vec![Instruction {
        program_id: PROGRAM_ID,
        accounts: account_metas.to_vec(),
        data,
    }])
}

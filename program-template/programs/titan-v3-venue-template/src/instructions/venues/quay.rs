use anchor_lang::{prelude::*, solana_program::instruction::Instruction};

/// Quay's mainnet program id.
pub const PROGRAM_ID: Pubkey = pubkey!("QUayE6nexQWYNZAEqfN8FxoNwQDSu3CAzT2qq9J1ArG");

/// `swap` instruction discriminator (`quay_sdk::consts::DISC_SWAP`).
const SWAP_DISCRIMINATOR: u8 = 0x20;

/// Build Quay's exact-in `swap` instruction for one route leg.
///
/// `account_metas` is the leg's account list, already assembled by the
/// off-chain route builder (`swap_route::build_swap_leg`) to match
/// `QuayVenue::generate_swap_instruction`: `[config, strategy, mm, quotes,
/// vault_in, vault_out, ata_in, ata_out, taker(TitanPDA), mint_in, mint_out,
/// instructions_sysvar, token_program(s)]`, with TitanPDA's signer flag managed
/// by the dispatcher. We pass them through unchanged and only encode the data.
///
/// Data layout: `[disc(0x20), amount_in(u64 LE), min_amount_out(u64 LE),
/// side(u8)]`. `min_amount_out` is 0 — Titan enforces slippage at the route
/// level. `side` is 0 (sell base) when `sell_base`, else 1 (buy base), and must
/// agree with the in/out account order the builder produced.
pub fn swap(
    sell_base: bool,
    amount_in: u64,
    account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
    let side: u8 = if sell_base { 0 } else { 1 };

    let mut data = Vec::with_capacity(1 + 8 + 8 + 1);
    data.push(SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.push(side);

    Ok(vec![Instruction {
        program_id: PROGRAM_ID,
        accounts: account_metas.to_vec(),
        data,
    }])
}

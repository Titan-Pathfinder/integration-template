use anchor_lang::{prelude::*, solana_program::instruction::Instruction};

// FILL_IN: replace this with your venue program id.
pub const TEMPLATE_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");

// FILL_IN: replace this with your venue's real swap discriminator.
const SWAP_DISCRIMINATOR: &[u8] = &[0];

// `zero_for_one` is only an example CPI parameter. If your venue needs more
// parameters, add them to the `Venue` enum variant and pass them from
// `perform_cpi_swap`.
pub fn swap(
    zero_for_one: bool,
    amount_in: u64,
    account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
    if TEMPLATE_PROGRAM_ID == Pubkey::default() {
        todo!("replace TEMPLATE_PROGRAM_ID with your venue program id")
    }
    if SWAP_DISCRIMINATOR == &[0] {
        todo!("replace SWAP_DISCRIMINATOR with your venue's real swap discriminator")
    }

    // FILL_IN: replace this serialization with your venue's swap instruction layout.
    let _ = (zero_for_one, amount_in, account_metas);
    todo!("replace template swap serialization with your venue's instruction layout")
}

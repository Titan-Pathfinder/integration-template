use anchor_lang::prelude::*;

use crate::state::TitanPda;

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(init, space = 8 + TitanPda::SIZE, seeds = [TitanPda::SEED], bump, payer = payer)]
    pub titan_pda: Account<'info, TitanPda>,
    pub system_program: Program<'info, System>,
}

impl Initialize<'_> {
    pub fn execute(ctx: Context<Initialize>) -> Result<()> {
        ctx.accounts.titan_pda.bump = ctx.bumps.titan_pda;
        Ok(())
    }
}

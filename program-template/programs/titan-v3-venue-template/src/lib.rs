use anchor_lang::prelude::*;

pub mod error;
pub mod instructions;
pub mod state;

use crate::instructions::*;

declare_id!("T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT");

#[program]
pub mod titan_v3_venue_template {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        Initialize::execute(ctx)
    }

    #[instruction(discriminator = [42])]
    pub fn swap_route_v3<'info>(
        ctx: Context<'_, '_, 'info, 'info, SwapRouteV3<'info>>,
        amount: u64,
        mints: u8,
        swaps: Vec<state::SwapSpecInputV2>,
    ) -> Result<()> {
        SwapRouteV3::execute(ctx, amount, mints, swaps)
    }
}

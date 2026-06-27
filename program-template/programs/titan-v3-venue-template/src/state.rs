use anchor_lang::prelude::*;

pub const MAX_SWAPS: usize = 12;
pub const MAX_MINTS: usize = 12;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Copy, Eq, Debug)]
pub enum Venue {
    /// A Quay swap leg. `sell_base` is Quay's `side` (true = sell base / side 0,
    /// false = buy base / side 1), encoded into the swap instruction's trailing
    /// `side` byte. Must match the route-builder `Venue` enum (same variants,
    /// same order — see the enum-parity test).
    Quay { sell_base: bool },
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Copy, Eq, Debug)]
pub struct SwapSpecInputV2 {
    pub venue: Venue,
    pub from: u8,
    pub to: u8,
    pub weight_nanos: u32,
    pub n_accounts: u8,
}

#[account]
pub struct TitanPda {
    pub bump: u8,
}

impl TitanPda {
    pub const SIZE: usize = 1;
    pub const SEED: &'static [u8] = b"titan_pda";
}

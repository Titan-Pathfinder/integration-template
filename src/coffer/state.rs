//! Borsh mirror of the on-chain `CofferPool` account
//! (`coffer-pool/src/state/coffer_pool.rs`, branch `main`).
//!
//! The contract declares the account with Anchor's `#[account]`, which is
//! Borsh plus an 8-byte discriminator. The structs below restate the SAME
//! fields in the SAME order with plain `borsh` derives; the byte layout is
//! therefore identical (1683 bytes including the discriminator).
//! `tests/coffer_source_parity.rs` compares the field lists against the
//! contract source so a layout change upstream fails loudly here.
//!
//! Do not trust `idl/coffer_pool.json` in the contracts repo for this layout —
//! it is stale. The source of truth is `state/coffer_pool.rs`.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_pubkey::Pubkey;

use crate::coffer::constants::MAX_TOKENS;
use crate::coffer::prelude::{Result, require};

/// `sha256("account:CofferPool")[..8]`.
pub const COFFER_POOL_DISCRIMINATOR: [u8; 8] = [137, 210, 42, 22, 209, 156, 43, 78];

/// Admin-controlled per-token configuration (88 bytes).
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetConfig {
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub normalized_weight: u64,
    pub max_selloff_pct: u16,
    pub max_selloff_period_length: u32,
    pub variable_fee_threshold_pct: u16,
    pub variable_fee_slope_low_pct: u16,
    pub variable_fee_slope_high_pct: u16,
    pub is_active: bool,
    pub variable_fee_slope_mid_pct: u16,
    pub variable_fee_kink_pct: u8,
}

impl Default for AssetConfig {
    fn default() -> Self {
        Self {
            mint: Pubkey::default(),
            token_program: Pubkey::default(),
            normalized_weight: 0,
            max_selloff_pct: 0,
            max_selloff_period_length: 0,
            variable_fee_threshold_pct: 0,
            variable_fee_slope_low_pct: 0,
            variable_fee_slope_high_pct: 0,
            is_active: true,
            variable_fee_slope_mid_pct: 0,
            variable_fee_kink_pct: 0,
        }
    }
}

impl AssetConfig {
    pub const LEN: usize = 32 + 32 + 8 + 2 + 4 + 2 + 2 + 2 + 1 + 3;
}

/// Swap/liquidity-updated per-token state (56 bytes).
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetDynamics {
    pub virtual_balance: u64,
    pub actual_balance: u64,
    pub protocol_fees_owed: u64,
    pub previous_selloff: u64,
    pub current_selloff: u64,
    pub window_start_timestamp: i64,
    pub selloff_vb_snapshot: u64,
}

impl AssetDynamics {
    pub const LEN: usize = 8 * 6 + 8;
}

/// One token slot (144 bytes).
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenSlot {
    pub config: AssetConfig,
    pub dynamics: AssetDynamics,
}

impl TokenSlot {
    pub const LEN: usize = AssetConfig::LEN + AssetDynamics::LEN;
}

/// The pool account body (everything after the discriminator).
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CofferPool {
    pub config: Pubkey,
    pub bump: u8,
    pub token_count: u8,
    pub pool_id: u64,
    pub swap_fee_rate: u32,
    pub protocol_fee_rate: u16,
    pub created_at: i64,
    pub pool_enabled: bool,
    pub swaps_enabled: bool,
    pub pool_admin: Pubkey,
    pub pending_pool_admin: Pubkey,
    pub range_manager: Pubkey,
    pub range_manager_enabled: bool,
    pub range_manager_max_vb_change_pct: u16,
    pub range_manager_max_weight_change_pct: u16,
    pub range_manager_min_update_interval_secs: u32,
    pub range_manager_last_updated: i64,
    pub tokens: [TokenSlot; MAX_TOKENS],
    pub lookup_table: Pubkey,
    pub banned_extensions: u64,
    pub range_manager_max_leverage_bps: u32,
    pub range_manager_min_leverage_bps: u32,
    pub reserved: [u8; 16],
}

impl Default for CofferPool {
    fn default() -> Self {
        Self {
            config: Pubkey::default(),
            bump: 0,
            token_count: 0,
            pool_id: 0,
            swap_fee_rate: 0,
            protocol_fee_rate: 0,
            created_at: 0,
            pool_enabled: true,
            swaps_enabled: true,
            pool_admin: Pubkey::default(),
            pending_pool_admin: Pubkey::default(),
            range_manager: Pubkey::default(),
            range_manager_enabled: false,
            range_manager_max_vb_change_pct: 0,
            range_manager_max_weight_change_pct: 0,
            range_manager_min_update_interval_secs: 0,
            range_manager_last_updated: 0,
            tokens: [TokenSlot::default(); MAX_TOKENS],
            lookup_table: Pubkey::default(),
            banned_extensions: 0,
            range_manager_max_leverage_bps: 0,
            range_manager_min_leverage_bps: 0,
            reserved: [0u8; 16],
        }
    }
}

/// Failure modes of [`CofferPool::from_account_data`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolDecodeError {
    /// `data.len() != CofferPool::LEN` — a v3 (1154-byte) pool that was never
    /// migrated, or not a pool at all.
    WrongLength(usize),
    /// The first eight bytes are not the `CofferPool` discriminator.
    WrongDiscriminator,
    /// Borsh rejected the body (e.g. a `bool` byte that is neither 0 nor 1).
    Borsh,
}

impl core::fmt::Display for PoolDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PoolDecodeError::WrongLength(n) => write!(
                f,
                "coffer pool account has {n} bytes, expected {}",
                CofferPool::LEN
            ),
            PoolDecodeError::WrongDiscriminator => write!(f, "not a CofferPool account"),
            PoolDecodeError::Borsh => write!(f, "coffer pool body failed to deserialize"),
        }
    }
}

impl CofferPool {
    /// Body size without the discriminator (1675).
    pub const INIT_SPACE: usize = 32
        + 1
        + 1
        + 8
        + 4
        + 2
        + 8
        + 1
        + 1
        + 32
        + 32
        + 32
        + 1
        + 2
        + 2
        + 4
        + 8
        + (MAX_TOKENS * TokenSlot::LEN)
        + 32
        + 8
        + 4
        + 20;

    /// Total on-chain size including the 8-byte discriminator (1683).
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Strictly decode a raw account: exact length, discriminator, then Borsh.
    pub fn from_account_data(data: &[u8]) -> core::result::Result<Self, PoolDecodeError> {
        if data.len() != Self::LEN {
            return Err(PoolDecodeError::WrongLength(data.len()));
        }
        if data[..8] != COFFER_POOL_DISCRIMINATOR {
            return Err(PoolDecodeError::WrongDiscriminator);
        }
        let mut body = &data[8..];
        let pool = CofferPool::deserialize(&mut body).map_err(|_| PoolDecodeError::Borsh)?;
        if !body.is_empty() {
            return Err(PoolDecodeError::Borsh);
        }
        Ok(pool)
    }

    /// Serialize back into the on-chain wire format (discriminator + body).
    /// Used by the fixture tests to build synthetic pool accounts.
    pub fn to_account_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::LEN);
        out.extend_from_slice(&COFFER_POOL_DISCRIMINATOR);
        self.serialize(&mut out)
            .expect("serializing into a Vec is infallible");
        debug_assert_eq!(out.len(), Self::LEN);
        out
    }

    /// Vault = ATA(pool, mint, token_program), exactly as the contract's
    /// `derive_vault`.
    pub fn derive_vault(pool_key: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
        let (ata, _) = Pubkey::find_program_address(
            &[pool_key.as_ref(), token_program.as_ref(), mint.as_ref()],
            &spl_associated_token_account::ID,
        );
        ata
    }

    /// The pool PDA for `(config, pool_id)` under `program_id`.
    pub fn derive_pool_address(config: &Pubkey, pool_id: u64, program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                crate::coffer::constants::COFFER_POOL_SEED,
                config.as_ref(),
                &pool_id.to_le_bytes(),
            ],
            program_id,
        )
    }

    /// Accumulate protocol fees for one token.
    pub fn add_protocol_fees(&mut self, token_index: usize, amount: u64) -> Result<()> {
        require!(
            token_index < self.token_count as usize,
            crate::coffer::errors::ErrorCode::InvalidTokenIndex,
        );
        let d = &mut self.tokens[token_index].dynamics;
        d.protocol_fees_owed = d
            .protocol_fees_owed
            .checked_add(amount)
            .ok_or(crate::coffer::errors::ErrorCode::MathOverflow)?;
        Ok(())
    }

    /// Active slots (`0..token_count`).
    pub fn active_slots(&self) -> &[TokenSlot] {
        &self.tokens[..(self.token_count as usize).min(MAX_TOKENS)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_contract() {
        assert_eq!(AssetConfig::LEN, 88);
        assert_eq!(AssetDynamics::LEN, 56);
        assert_eq!(TokenSlot::LEN, 144);
        assert_eq!(CofferPool::INIT_SPACE, 1675);
        assert_eq!(CofferPool::LEN, 1683);
    }

    #[test]
    fn round_trips_through_the_wire_format() {
        let mut pool = CofferPool {
            token_count: 3,
            swap_fee_rate: 3_000,
            ..Default::default()
        };
        pool.tokens[2].config.variable_fee_kink_pct = 77;
        pool.tokens[2].dynamics.selloff_vb_snapshot = 42;
        pool.range_manager_min_leverage_bps = 9;
        let bytes = pool.to_account_data();
        assert_eq!(bytes.len(), CofferPool::LEN);
        let back = CofferPool::from_account_data(&bytes).unwrap();
        assert_eq!(back, pool);
    }

    #[test]
    fn field_offsets_match_the_documented_layout() {
        // Spot-check offsets by writing distinctive values and reading bytes.
        let mut pool = CofferPool::default();
        pool.token_count = 7;
        pool.swap_fee_rate = 0x0102_0304;
        pool.tokens[0].config.normalized_weight = 0x1111;
        pool.tokens[0].config.max_selloff_pct = 0x2222;
        pool.tokens[0].config.variable_fee_kink_pct = 0x33;
        pool.tokens[0].dynamics.virtual_balance = 0x4444;
        pool.tokens[0].dynamics.selloff_vb_snapshot = 0x5555;
        pool.tokens[1].config.mint = Pubkey::new_from_array([9u8; 32]);
        pool.range_manager_max_leverage_bps = 0x6666;
        let b = pool.to_account_data();
        assert_eq!(b[41], 7);
        assert_eq!(
            u32::from_le_bytes(b[50..54].try_into().unwrap()),
            0x0102_0304
        );
        const TOKENS: usize = 8 + 171;
        assert_eq!(
            u64::from_le_bytes(b[TOKENS + 64..TOKENS + 72].try_into().unwrap()),
            0x1111
        );
        assert_eq!(
            u16::from_le_bytes(b[TOKENS + 72..TOKENS + 74].try_into().unwrap()),
            0x2222
        );
        assert_eq!(b[TOKENS + 87], 0x33);
        assert_eq!(
            u64::from_le_bytes(b[TOKENS + 88..TOKENS + 96].try_into().unwrap()),
            0x4444
        );
        assert_eq!(
            u64::from_le_bytes(b[TOKENS + 136..TOKENS + 144].try_into().unwrap()),
            0x5555
        );
        assert_eq!(&b[TOKENS + 144..TOKENS + 176], &[9u8; 32]);
        const LOOKUP: usize = TOKENS + 10 * 144;
        assert_eq!(LOOKUP, 1619);
        assert_eq!(
            u32::from_le_bytes(b[1659..1663].try_into().unwrap()),
            0x6666
        );
    }

    #[test]
    fn rejects_wrong_length_and_discriminator() {
        assert_eq!(
            CofferPool::from_account_data(&[0u8; 1154]),
            Err(PoolDecodeError::WrongLength(1154))
        );
        let mut bytes = CofferPool::default().to_account_data();
        bytes[0] ^= 1;
        assert_eq!(
            CofferPool::from_account_data(&bytes),
            Err(PoolDecodeError::WrongDiscriminator)
        );
        let mut bytes = CofferPool::default().to_account_data();
        bytes[64] = 2; // pool_enabled must be 0 or 1
        assert_eq!(
            CofferPool::from_account_data(&bytes),
            Err(PoolDecodeError::Borsh)
        );
    }
}

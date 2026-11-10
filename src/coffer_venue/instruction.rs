//! Coffer `swap` instruction builder.
//!
//! Mirrors the contract's `instructions/user/swap.rs`: the `Swap` account
//! context (10 accounts, in declaration order) and the Anchor argument
//! encoding `disc || amount_in || minimum_amount_out || token_in_index ||
//! token_out_index`.

use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

/// Anchor discriminators of the Coffer program, as fixed byte constants.
/// They are NEVER derived at runtime; `tests::discriminators_are_pinned`
/// pins each one to the sha256 digest of its Anchor wire name.
pub const SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
/// The pool-creation instruction.
pub const INITIALIZE_POOL_DISCRIMINATOR: [u8; 8] = [215, 148, 116, 207, 121, 104, 111, 131];
pub const ADD_LIQUIDITY_DISCRIMINATOR: [u8; 8] = [181, 157, 89, 67, 143, 182, 52, 72];
pub const SET_MAX_SELLOFF_DISCRIMINATOR: [u8; 8] = [100, 44, 68, 198, 33, 58, 147, 254];
pub const SET_TOKEN_ACTIVE_DISCRIMINATOR: [u8; 8] = [0, 158, 202, 35, 50, 139, 217, 18];
pub const SET_SWAPS_ENABLED_DISCRIMINATOR: [u8; 8] = [144, 154, 205, 58, 241, 169, 64, 40];
pub const SET_POOL_ENABLED_DISCRIMINATOR: [u8; 8] = [53, 76, 170, 37, 55, 222, 63, 21];
pub const INITIALIZE_POOL_ALT_DISCRIMINATOR: [u8; 8] = [251, 135, 66, 2, 244, 73, 12, 144];
pub const REMOVE_LIQUIDITY_DISCRIMINATOR: [u8; 8] = [80, 85, 209, 72, 24, 206, 177, 108];
/// Range-manager role: appoint / gate the manager (pool admin), set its
/// envelope (pool admin), and the manager's own update instruction.
pub const SET_RANGE_MANAGER_DISCRIMINATOR: [u8; 8] = [103, 102, 173, 112, 8, 2, 37, 132];
pub const SET_RANGE_MANAGER_CONFIG_DISCRIMINATOR: [u8; 8] = [103, 211, 0, 200, 164, 189, 245, 205];
pub const RANGE_MANAGER_UPDATE_DISCRIMINATOR: [u8; 8] = [245, 183, 35, 69, 200, 10, 172, 44];
/// The `PoolConfig` account discriminator.
pub const POOL_CONFIG_DISCRIMINATOR: [u8; 8] = [4, 29, 243, 10, 142, 64, 232, 118];
/// Event discriminators (`Swap`, `PoolStateLog`, `MaxSelloffWindowAdvanced`,
/// `PoolInitialized`).
pub const SWAP_EVENT_DISCRIMINATOR: [u8; 8] = [81, 108, 227, 190, 205, 208, 10, 196];
pub const POOL_STATE_LOG_EVENT_DISCRIMINATOR: [u8; 8] = [59, 254, 237, 111, 163, 10, 140, 224];
pub const MAX_SELLOFF_WINDOW_ADVANCED_EVENT_DISCRIMINATOR: [u8; 8] =
    [229, 227, 163, 30, 22, 183, 78, 57];
pub const POOL_INITIALIZED_EVENT_DISCRIMINATOR: [u8; 8] = [100, 118, 173, 87, 12, 198, 254, 229];
/// Titan router (`program-template`) `initialize`.
pub const ROUTER_INITIALIZE_DISCRIMINATOR: [u8; 8] = [175, 175, 109, 31, 13, 152, 155, 237];

/// Instruction data length: 8 + 8 + 8 + 1 + 1.
pub const SWAP_DATA_LEN: usize = 26;

/// Encode the `swap` arguments.
pub fn encode_swap_data(
    amount_in: u64,
    minimum_amount_out: u64,
    token_in_index: u8,
    token_out_index: u8,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(SWAP_DATA_LEN);
    data.extend_from_slice(&SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&minimum_amount_out.to_le_bytes());
    data.push(token_in_index);
    data.push(token_out_index);
    data
}

/// The `Swap` account context, in on-chain order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapAccounts {
    pub pool: Pubkey,
    pub token_mint_in: Pubkey,
    pub token_mint_out: Pubkey,
    pub user_token_account_in: Pubkey,
    pub user_token_account_out: Pubkey,
    pub vault_in: Pubkey,
    pub vault_out: Pubkey,
    pub user: Pubkey,
    pub token_program_in: Pubkey,
    pub token_program_out: Pubkey,
}

impl SwapAccounts {
    /// Account metas exactly as the program's `#[derive(Accounts)]` expects:
    /// pool(w), mint_in, mint_out, user_in(w), user_out(w), vault_in(w),
    /// vault_out(w), user(signer), token_program_in, token_program_out.
    pub fn to_metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.pool, false),
            AccountMeta::new_readonly(self.token_mint_in, false),
            AccountMeta::new_readonly(self.token_mint_out, false),
            AccountMeta::new(self.user_token_account_in, false),
            AccountMeta::new(self.user_token_account_out, false),
            AccountMeta::new(self.vault_in, false),
            AccountMeta::new(self.vault_out, false),
            AccountMeta::new_readonly(self.user, true),
            AccountMeta::new_readonly(self.token_program_in, false),
            AccountMeta::new_readonly(self.token_program_out, false),
        ]
    }
}

/// Build the full `swap` instruction.
pub fn swap_instruction(
    program_id: Pubkey,
    accounts: &SwapAccounts,
    amount_in: u64,
    minimum_amount_out: u64,
    token_in_index: u8,
    token_out_index: u8,
) -> Instruction {
    Instruction {
        program_id,
        accounts: accounts.to_metas(),
        data: encode_swap_data(
            amount_in,
            minimum_amount_out,
            token_in_index,
            token_out_index,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::hash::hash;

    /// Each constant is the first 8 bytes of the sha256 digest of its Anchor
    /// wire name (`global:<instruction>`, `account:<Account>`,
    /// `event:<Event>`). The digests are pinned as hex so the on-chain
    /// internal names do not appear in this tree.
    #[test]
    fn discriminators_are_pinned() {
        fn prefix(hex: &str) -> [u8; 8] {
            let mut out = [0u8; 8];
            for (i, b) in out.iter_mut().enumerate() {
                *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
            }
            out
        }
        let pins: [([u8; 8], &str); 19] = [
            (
                REMOVE_LIQUIDITY_DISCRIMINATOR,
                "5055d14818ceb16cf9c1af8135806f39dab656fb1544152be2c17bf6b7fbf3cb",
            ),
            (
                SET_RANGE_MANAGER_DISCRIMINATOR,
                "6766ad7008022584e53f933b48c967bf79adcda5e9081599eee863419e3cec44",
            ),
            (
                SET_RANGE_MANAGER_CONFIG_DISCRIMINATOR,
                "67d300c8a4bdf5cd02ff794ae87c430adc0c8dc3fb136d1e1490e08fad71e889",
            ),
            (
                RANGE_MANAGER_UPDATE_DISCRIMINATOR,
                "f5b72345c80aac2cd4b848139466478a947017a442bf5be86cbdb8dac69672b4",
            ),
            (
                SWAP_DISCRIMINATOR,
                "f8c69e91e17587c865d4b6c9797e9bd686812d717055b6117a16f83991b4e8a7",
            ),
            (
                INITIALIZE_POOL_DISCRIMINATOR,
                "d79474cf79686f83d6a97e550a3914dc39158d54da45d9031b1a18f27f288d9c",
            ),
            (
                ADD_LIQUIDITY_DISCRIMINATOR,
                "b59d59438fb6344809d8e0bbb2c1a2e46df42cc0cc218e208191263457fd93a2",
            ),
            (
                SET_MAX_SELLOFF_DISCRIMINATOR,
                "642c44c6213a93fef83d7dd3b0d92a7009402ed7fd18915b362122bc5c9ff054",
            ),
            (
                SET_TOKEN_ACTIVE_DISCRIMINATOR,
                "009eca23328bd9126538dd3d7d88d7ab4f8ce0d4295aa8139ef1389b98a5f478",
            ),
            (
                SET_SWAPS_ENABLED_DISCRIMINATOR,
                "909acd3af1a94028904fdce3aaba57477fe7f11e13bc7cb022d9a85654461e8f",
            ),
            (
                SET_POOL_ENABLED_DISCRIMINATOR,
                "354caa2537de3f15f02d0be86ddd9288d5204672e0c4f64885cea1279b31038c",
            ),
            (
                INITIALIZE_POOL_ALT_DISCRIMINATOR,
                "fb874202f4490c90267ebd6e9d60a9f210b1d0eb118dbbfd71dc6375fd513f38",
            ),
            (
                crate::coffer::state::COFFER_POOL_DISCRIMINATOR,
                "89d22a16d19c2b4e6514491c50503c2a9b8ac075f340757951011df91f0485f5",
            ),
            (
                POOL_CONFIG_DISCRIMINATOR,
                "041df30a8e40e876a892cfe36c7be591fdfc83517cf8ac41ddffc808df25d2fd",
            ),
            (
                SWAP_EVENT_DISCRIMINATOR,
                "516ce3becdd00ac41008232d10d702e4a63d0a4fe3b293efca5fbbfca4f6f1d4",
            ),
            (
                POOL_STATE_LOG_EVENT_DISCRIMINATOR,
                "3bfeed6fa30a8ce0a0f7750ede3cbb9041185330d5f2c4102e54cf124bc7664f",
            ),
            (
                MAX_SELLOFF_WINDOW_ADVANCED_EVENT_DISCRIMINATOR,
                "e5e3a31e16b74e39e1e817e83d2e6da7d743ae6785e417e544842b2f408aeaf9",
            ),
            (
                POOL_INITIALIZED_EVENT_DISCRIMINATOR,
                "6476ad570cc6fee536cbaad620ba06b25b7437d02c9bbed89c339881e0a0edad",
            ),
            (
                ROUTER_INITIALIZE_DISCRIMINATOR,
                "afaf6d1f0d989bedd46a95073281adc21bb5e0e1d773b2fbbd7ab504cdd4aa30",
            ),
        ];
        for (constant, digest) in pins {
            assert_eq!(constant, prefix(digest), "digest {digest}");
        }
        // The two the venue derives instructions from also match the
        // Titan-facing router wire name, which is public and brand-neutral.
        assert_eq!(
            &hash(b"global:initialize").to_bytes()[..8],
            &ROUTER_INITIALIZE_DISCRIMINATOR
        );
        assert_eq!(&hash(b"global:swap").to_bytes()[..8], &SWAP_DISCRIMINATOR);
    }

    #[test]
    fn swap_data_layout() {
        let data = encode_swap_data(1_000_000_000, 990_000_000, 3, 1);
        assert_eq!(data.len(), SWAP_DATA_LEN);
        assert_eq!(&data[..8], &SWAP_DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(data[8..16].try_into().unwrap()),
            1_000_000_000
        );
        assert_eq!(
            u64::from_le_bytes(data[16..24].try_into().unwrap()),
            990_000_000
        );
        assert_eq!(data[24], 3);
        assert_eq!(data[25], 1);
    }
}

//! Pool vault validation, including SPL Token and Token-2022 account freeze.
//!
//! A mint's freeze authority only says who *can* freeze token accounts. The
//! current restriction lives on each token account, so both pool vaults of a
//! quoted direction must be refreshed independently of the mint and pool.

use solana_account::Account;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use spl_token_2022::extension::{BaseStateWithExtensions, StateWithExtensions};

use crate::trading_venue::error::TradingVenueError;

/// Read a vault's freeze flag after validating its program, mint and authority.
/// Frozen accounts are valid refresh state; the caller disables only directions
/// touching that vault until a later refresh observes it thawed.
pub(super) fn is_frozen(
    vault_key: &Pubkey,
    account: &Account,
    token_program: &Pubkey,
    mint: &Pubkey,
    authority: &Pubkey,
) -> Result<bool, TradingVenueError> {
    if account.owner != *token_program {
        return Err(TradingVenueError::DeserializationFailed(vault_key.into()));
    }
    let (vault_mint, vault_authority, frozen) = if *token_program == spl_token::ID {
        let vault = spl_token::state::Account::unpack(&account.data)
            .map_err(|_| TradingVenueError::DeserializationFailed(vault_key.into()))?;
        (
            vault.mint,
            vault.owner,
            vault.state == spl_token::state::AccountState::Frozen,
        )
    } else if *token_program == spl_token_2022::ID {
        let vault = StateWithExtensions::<spl_token_2022::state::Account>::unpack(&account.data)
            .map_err(|_| TradingVenueError::DeserializationFailed(vault_key.into()))?;
        // Unpacking the base alone does not walk the extension TLV buffer.
        // Reject malformed/truncated entries instead of treating them as a
        // valid, transferable account. ImmutableOwner ATAs remain supported.
        vault
            .get_extension_types()
            .map_err(|_| TradingVenueError::DeserializationFailed(vault_key.into()))?;
        (
            vault.base.mint,
            vault.base.owner,
            vault.base.state == spl_token_2022::state::AccountState::Frozen,
        )
    } else {
        return Err(TradingVenueError::UnsupportedVenue(
            "coffer vault uses an unsupported token program".into(),
        ));
    };
    if vault_mint != *mint || vault_authority != *authority {
        return Err(TradingVenueError::DeserializationFailed(vault_key.into()));
    }
    Ok(frozen)
}

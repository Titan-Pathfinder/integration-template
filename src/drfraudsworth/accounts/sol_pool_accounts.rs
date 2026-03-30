// AccountMeta builders for Tax Program SOL pool swap instructions.
//
// Exact account ordering of SwapSolBuy (20 named) and SwapSolSell (21 named).
// Transfer hook remaining_accounts appended (4 per T22 mint).

use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

use super::addresses::{
    AMM_PROGRAM_ID, CARNAGE_SOL_VAULT_PDA, CRIME_MINT, CRIME_SOL_POOL, CRIME_SOL_VAULT_A,
    CRIME_SOL_VAULT_B, EPOCH_STATE_PDA, ESCROW_VAULT_PDA, FRAUD_MINT, FRAUD_SOL_POOL,
    FRAUD_SOL_VAULT_A, FRAUD_SOL_VAULT_B, NATIVE_MINT, SPL_TOKEN_PROGRAM_ID, STAKING_PROGRAM_ID,
    STAKE_POOL_PDA, SWAP_AUTHORITY_PDA, SYSTEM_PROGRAM_ID, TAX_AUTHORITY_PDA, TOKEN_2022_PROGRAM_ID,
    TREASURY, WSOL_INTERMEDIARY_PDA,
};
use super::hook_accounts::hook_metas_for_mint;

/// Build the 20 named + 4 hook = 24 AccountMetas for SwapSolBuy (SOL -> token).
pub fn build_buy_account_metas(
    user: &Pubkey,
    user_wsol_ata: &Pubkey,
    user_token_ata: &Pubkey,
    is_crime: bool,
) -> Vec<AccountMeta> {
    let (pool, vault_a, vault_b, token_mint) = pool_addresses(is_crime);

    let mut metas = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(EPOCH_STATE_PDA, false),
        AccountMeta::new_readonly(SWAP_AUTHORITY_PDA, false),
        AccountMeta::new_readonly(TAX_AUTHORITY_PDA, false),
        AccountMeta::new(pool, false),
        AccountMeta::new(vault_a, false),
        AccountMeta::new(vault_b, false),
        AccountMeta::new_readonly(NATIVE_MINT, false),
        AccountMeta::new_readonly(token_mint, false),
        AccountMeta::new(*user_wsol_ata, false),
        AccountMeta::new(*user_token_ata, false),
        AccountMeta::new(STAKE_POOL_PDA, false),
        AccountMeta::new(ESCROW_VAULT_PDA, false),
        AccountMeta::new(CARNAGE_SOL_VAULT_PDA, false),
        AccountMeta::new(TREASURY, false),
        AccountMeta::new_readonly(AMM_PROGRAM_ID, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(TOKEN_2022_PROGRAM_ID, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(STAKING_PROGRAM_ID, false),
    ];

    let hook_metas = hook_metas_for_mint(&token_mint, &vault_b, user_token_ata);
    metas.extend(hook_metas);

    metas
}

/// Build the 21 named + 4 hook = 25 AccountMetas for SwapSolSell (token -> SOL).
pub fn build_sell_account_metas(
    user: &Pubkey,
    user_token_ata: &Pubkey,
    user_wsol_ata: &Pubkey,
    is_crime: bool,
) -> Vec<AccountMeta> {
    let (pool, vault_a, vault_b, token_mint) = pool_addresses(is_crime);

    let mut metas = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(EPOCH_STATE_PDA, false),
        AccountMeta::new(SWAP_AUTHORITY_PDA, false),
        AccountMeta::new_readonly(TAX_AUTHORITY_PDA, false),
        AccountMeta::new(pool, false),
        AccountMeta::new(vault_a, false),
        AccountMeta::new(vault_b, false),
        AccountMeta::new_readonly(NATIVE_MINT, false),
        AccountMeta::new_readonly(token_mint, false),
        AccountMeta::new(*user_wsol_ata, false),
        AccountMeta::new(*user_token_ata, false),
        AccountMeta::new(STAKE_POOL_PDA, false),
        AccountMeta::new(ESCROW_VAULT_PDA, false),
        AccountMeta::new(CARNAGE_SOL_VAULT_PDA, false),
        AccountMeta::new(TREASURY, false),
        AccountMeta::new(WSOL_INTERMEDIARY_PDA, false),
        AccountMeta::new_readonly(AMM_PROGRAM_ID, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(TOKEN_2022_PROGRAM_ID, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(STAKING_PROGRAM_ID, false),
    ];

    let hook_metas = hook_metas_for_mint(&token_mint, user_token_ata, &vault_b);
    metas.extend(hook_metas);

    metas
}

fn pool_addresses(is_crime: bool) -> (Pubkey, Pubkey, Pubkey, Pubkey) {
    if is_crime {
        (CRIME_SOL_POOL, CRIME_SOL_VAULT_A, CRIME_SOL_VAULT_B, CRIME_MINT)
    } else {
        (FRAUD_SOL_POOL, FRAUD_SOL_VAULT_A, FRAUD_SOL_VAULT_B, FRAUD_MINT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buy_metas_has_24_accounts() {
        let metas = build_buy_account_metas(
            &Pubkey::new_unique(), &Pubkey::new_unique(), &Pubkey::new_unique(), true,
        );
        assert_eq!(metas.len(), 24);
    }

    #[test]
    fn sell_metas_has_25_accounts() {
        let metas = build_sell_account_metas(
            &Pubkey::new_unique(), &Pubkey::new_unique(), &Pubkey::new_unique(), true,
        );
        assert_eq!(metas.len(), 25);
    }

    #[test]
    fn sell_swap_authority_is_mutable() {
        let metas = build_sell_account_metas(
            &Pubkey::new_unique(), &Pubkey::new_unique(), &Pubkey::new_unique(), true,
        );
        assert_eq!(metas[2].pubkey, SWAP_AUTHORITY_PDA);
        assert!(metas[2].is_writable);
    }

    #[test]
    fn buy_swap_authority_is_readonly() {
        let metas = build_buy_account_metas(
            &Pubkey::new_unique(), &Pubkey::new_unique(), &Pubkey::new_unique(), true,
        );
        assert_eq!(metas[2].pubkey, SWAP_AUTHORITY_PDA);
        assert!(!metas[2].is_writable);
    }
}

// AccountMeta builder for Conversion Vault convert_v2 instruction.
//
// 9 named + 8 hook = 17 AccountMetas total.

use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

use super::addresses::{
    CRIME_MINT, FRAUD_MINT, PROFIT_MINT, TOKEN_2022_PROGRAM_ID,
    VAULT_CONFIG_PDA, VAULT_CRIME, VAULT_FRAUD, VAULT_PROFIT,
};
use super::hook_accounts::hook_metas_for_mint;

/// Build the 9 named + 8 hook = 17 AccountMetas for convert_v2.
pub fn build_vault_account_metas(
    user: &Pubkey,
    user_input_ata: &Pubkey,
    user_output_ata: &Pubkey,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
) -> Vec<AccountMeta> {
    let vault_input = vault_token_account(input_mint);
    let vault_output = vault_token_account(output_mint);

    let mut metas = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(VAULT_CONFIG_PDA, false),
        AccountMeta::new(*user_input_ata, false),
        AccountMeta::new(*user_output_ata, false),
        AccountMeta::new_readonly(*input_mint, false),
        AccountMeta::new_readonly(*output_mint, false),
        AccountMeta::new(vault_input, false),
        AccountMeta::new(vault_output, false),
        AccountMeta::new_readonly(TOKEN_2022_PROGRAM_ID, false),
    ];

    let input_hooks = hook_metas_for_mint(input_mint, user_input_ata, &vault_input);
    let output_hooks = hook_metas_for_mint(output_mint, &vault_output, user_output_ata);

    metas.extend(input_hooks);
    metas.extend(output_hooks);

    metas
}

pub fn vault_token_account(mint: &Pubkey) -> Pubkey {
    if *mint == CRIME_MINT {
        VAULT_CRIME
    } else if *mint == FRAUD_MINT {
        VAULT_FRAUD
    } else if *mint == PROFIT_MINT {
        VAULT_PROFIT
    } else {
        panic!("vault_token_account: unknown mint {}", mint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_metas_has_17_accounts() {
        let metas = build_vault_account_metas(
            &Pubkey::new_unique(), &Pubkey::new_unique(), &Pubkey::new_unique(),
            &CRIME_MINT, &PROFIT_MINT,
        );
        assert_eq!(metas.len(), 17);
    }

    #[test]
    fn vault_token_account_maps_correctly() {
        assert_eq!(vault_token_account(&CRIME_MINT), VAULT_CRIME);
        assert_eq!(vault_token_account(&FRAUD_MINT), VAULT_FRAUD);
        assert_eq!(vault_token_account(&PROFIT_MINT), VAULT_PROFIT);
    }
}

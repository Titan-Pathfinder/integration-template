// Transfer hook extra account metas for Token-2022 mints.
//
// Each T22 mint requires 4 extra accounts for transfer_checked:
//   1. ExtraAccountMetaList PDA (readonly)
//   2. Whitelist source PDA (readonly)
//   3. Whitelist dest PDA (readonly)
//   4. Transfer Hook Program (readonly)
//
// For NATIVE_MINT (SPL Token, no hooks), returns empty vec.

use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

use super::addresses::{
    CRIME_HOOK_META, CRIME_MINT, FRAUD_HOOK_META, FRAUD_MINT, NATIVE_MINT, PROFIT_HOOK_META,
    PROFIT_MINT, TRANSFER_HOOK_PROGRAM_ID,
};

pub fn hook_metas_for_mint(
    mint: &Pubkey,
    source_token_account: &Pubkey,
    dest_token_account: &Pubkey,
) -> Vec<AccountMeta> {
    if *mint == NATIVE_MINT {
        return vec![];
    }

    let meta_list = if *mint == CRIME_MINT {
        CRIME_HOOK_META
    } else if *mint == FRAUD_MINT {
        FRAUD_HOOK_META
    } else if *mint == PROFIT_MINT {
        PROFIT_HOOK_META
    } else {
        return vec![];
    };

    let (wl_source, _) = Pubkey::find_program_address(
        &[b"whitelist", source_token_account.as_ref()],
        &TRANSFER_HOOK_PROGRAM_ID,
    );
    let (wl_dest, _) = Pubkey::find_program_address(
        &[b"whitelist", dest_token_account.as_ref()],
        &TRANSFER_HOOK_PROGRAM_ID,
    );

    vec![
        AccountMeta::new_readonly(meta_list, false),
        AccountMeta::new_readonly(wl_source, false),
        AccountMeta::new_readonly(wl_dest, false),
        AccountMeta::new_readonly(TRANSFER_HOOK_PROGRAM_ID, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_mint_returns_empty() {
        let metas = hook_metas_for_mint(&NATIVE_MINT, &Pubkey::new_unique(), &Pubkey::new_unique());
        assert!(metas.is_empty());
    }

    #[test]
    fn crime_mint_returns_4_metas() {
        let metas = hook_metas_for_mint(&CRIME_MINT, &Pubkey::new_unique(), &Pubkey::new_unique());
        assert_eq!(metas.len(), 4);
        assert_eq!(metas[0].pubkey, CRIME_HOOK_META);
        assert_eq!(metas[3].pubkey, TRANSFER_HOOK_PROGRAM_ID);
    }

    #[test]
    fn whitelist_pdas_are_deterministic() {
        let src = Pubkey::new_unique();
        let dst = Pubkey::new_unique();
        let m1 = hook_metas_for_mint(&CRIME_MINT, &src, &dst);
        let m2 = hook_metas_for_mint(&CRIME_MINT, &src, &dst);
        assert_eq!(m1[1].pubkey, m2[1].pubkey);
        assert_eq!(m1[2].pubkey, m2[2].pubkey);
    }
}

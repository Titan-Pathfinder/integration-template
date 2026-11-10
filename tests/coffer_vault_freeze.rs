//! Pool-vault freeze and refresh regression tests, independent of RPC state.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use async_trait::async_trait;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_compute_budget::compute_budget::ComputeBudget;
use solana_instruction::error::InstructionError;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::TransactionError};
use solana_sysvar::clock::{self, Clock};
use solana_transaction::Transaction;
use spl_token::state::{Account as TokenAccount, AccountState, Mint};
use spl_token_2022::extension::{
    BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
    immutable_owner::ImmutableOwner,
};
use titan_integration_template::{
    account_caching::{AccountCacheError, AccountsCache},
    coffer::{
        state::CofferPool,
        swap::{apply_swap, quote_exact_in},
    },
    coffer_venue::{COFFER_PROGRAM_ID, CofferVenue},
    trading_venue::{
        FromAccount, QuoteRequest, SwapType, TradingVenue,
        error::{ErrorInfo, TradingVenueError},
    },
};

struct Snapshot(HashMap<Pubkey, Account>);

#[async_trait]
impl AccountsCache for Snapshot {
    async fn get_account(&self, key: &Pubkey) -> Result<Option<Account>, AccountCacheError> {
        Ok(self.0.get(key).cloned())
    }

    async fn get_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<Account>>, AccountCacheError> {
        Ok(keys.iter().map(|key| self.0.get(key).cloned()).collect())
    }
}

struct Fixture {
    cache: Snapshot,
    venue: CofferVenue,
    mints: [Pubkey; 3],
    vaults: [Pubkey; 3],
}

impl Fixture {
    async fn new(program: Pubkey, extended: bool) -> Self {
        let config = Pubkey::new_unique();
        let (pool_key, bump) = CofferPool::derive_pool_address(&config, 7, &COFFER_PROGRAM_ID);
        let mut pool = CofferPool {
            config,
            bump,
            pool_id: 7,
            token_count: 3,
            swap_fee_rate: 3_000,
            ..Default::default()
        };
        let mints = std::array::from_fn(|_| Pubkey::new_unique());
        let vaults =
            std::array::from_fn(|i| CofferPool::derive_vault(&pool_key, &mints[i], &program));
        let mut accounts = HashMap::new();
        for i in 0..3 {
            let token = &mut pool.tokens[i];
            token.config.mint = mints[i];
            token.config.token_program = program;
            token.config.normalized_weight = if i == 2 { 3_334 } else { 3_333 };
            token.dynamics.virtual_balance = 1_000_000_000_000;
            token.dynamics.actual_balance = 1_000_000_000_000;
            let mut mint = Account::new(1_000_000_000, Mint::LEN, &program);
            Mint {
                decimals: 9,
                is_initialized: true,
                // The authority's presence alone must not disable routing.
                freeze_authority: Some(Pubkey::new_unique()).into(),
                ..Default::default()
            }
            .pack_into_slice(&mut mint.data);
            accounts.insert(mints[i], mint);
            let mut vault = Account::new(1_000_000_000, TokenAccount::LEN, &program);
            if extended {
                vault.data.resize(
                    ExtensionType::try_calculate_account_len::<spl_token_2022::state::Account>(&[
                        ExtensionType::ImmutableOwner,
                    ])
                    .unwrap(),
                    0,
                );
                let mut state =
                    StateWithExtensionsMut::<spl_token_2022::state::Account>::unpack_uninitialized(
                        &mut vault.data,
                    )
                    .unwrap();
                state.init_extension::<ImmutableOwner>(true).unwrap();
                state.base = spl_token_2022::state::Account {
                    mint: mints[i],
                    owner: pool_key,
                    amount: token.dynamics.actual_balance,
                    state: spl_token_2022::state::AccountState::Initialized,
                    ..Default::default()
                };
                state.pack_base();
                state.init_account_type().unwrap();
            } else {
                TokenAccount {
                    mint: mints[i],
                    owner: pool_key,
                    amount: token.dynamics.actual_balance,
                    state: AccountState::Initialized,
                    ..Default::default()
                }
                .pack_into_slice(&mut vault.data);
            }
            accounts.insert(vaults[i], vault);
        }
        let pool_account = Account {
            lamports: 1_000_000_000,
            owner: COFFER_PROGRAM_ID,
            data: pool.to_account_data(),
            ..Default::default()
        };
        let venue = CofferVenue::from_account(&pool_key, &pool_account).unwrap();
        accounts.insert(pool_key, pool_account);
        accounts.insert(
            clock::ID,
            Account {
                data: bincode::serialize(&Clock {
                    unix_timestamp: 1_800_000_000,
                    ..Default::default()
                })
                .unwrap(),
                ..Default::default()
            },
        );
        let mut fixture = Self {
            cache: Snapshot(accounts),
            venue,
            mints,
            vaults,
        };
        fixture.refresh().await;
        fixture
    }

    async fn refresh(&mut self) {
        self.venue.update_state(&self.cache).await.unwrap();
        assert!(self.venue.initialized());
    }

    fn request(&self, input: usize, output: usize, amount: u64) -> QuoteRequest {
        QuoteRequest {
            input_mint: self.mints[input],
            output_mint: self.mints[output],
            amount,
            swap_type: SwapType::ExactIn,
        }
    }

    fn set_frozen(&mut self, slot: usize, frozen: bool) {
        let account = self.cache.0.get_mut(&self.vaults[slot]).unwrap();
        // Both programs have the same packed base; retain the extension tail.
        let mut token = TokenAccount::unpack(&account.data[..TokenAccount::LEN]).unwrap();
        token.state = if frozen {
            AccountState::Frozen
        } else {
            AccountState::Initialized
        };
        token.pack_into_slice(&mut account.data[..TokenAccount::LEN]);
    }
}

#[tokio::test]
async fn frozen_vault_blocks_both_sides_and_thaw_restores_same_venue() {
    for (program, extended) in [
        (spl_token::ID, false),
        (spl_token_2022::ID, false),
        (spl_token_2022::ID, true),
    ] {
        let mut f = Fixture::new(program, extended).await;
        let request = f.request(0, 1, 1_000_000);
        let before = f.venue.quote(request.clone()).unwrap();
        let declared = f.venue.directions_num();
        let keys = f.venue.get_required_pubkeys_for_update().unwrap();
        assert!(f.vaults.iter().all(|vault| keys.contains(vault)));
        assert_eq!(keys.len(), 8); // pool, three mints, three vaults, Clock

        for frozen_slot in [0, 1] {
            f.set_frozen(frozen_slot, true);
            f.refresh().await;
            assert_eq!(f.venue.directions_num(), declared);
            for (input, output) in [(0, 1), (1, 0)] {
                for amount in [0, 1_000_000, u64::MAX] {
                    assert!(matches!(
                        f.venue.quote(f.request(input, output, amount)),
                        Err(TradingVenueError::AmmMethodError(ErrorInfo::StaticStr(
                            "AccountFrozen"
                        )))
                    ));
                }
                assert!(f.venue.bounds(input as u8, output as u8).is_err());
            }
            // A different pair in the same pool remains available.
            let other_slot = 1 - frozen_slot;
            assert!(
                f.venue
                    .quote(f.request(other_slot, 2, 1_000_000))
                    .unwrap()
                    .expected_output
                    > 0
            );
            f.set_frozen(frozen_slot, false);
            f.refresh().await;
            let after = f.venue.quote(request.clone()).unwrap();
            assert_eq!(after.expected_output, before.expected_output);
            assert_eq!(after.price, before.price);
        }
    }
}

#[tokio::test]
async fn failed_vault_refresh_invalidates_old_quotes_and_recovers() {
    for (program, extended) in [(spl_token::ID, false), (spl_token_2022::ID, true)] {
        let mut f = Fixture::new(program, extended).await;
        let request = f.request(0, 1, 1_000_000);
        let original = f.cache.0[&f.vaults[0]].clone();
        // Missing, wrong program, wrong mint, wrong authority, uninitialized,
        // truncated base, plus a malformed Token-2022 extension length.
        let cases = if extended { 7 } else { 6 };
        for case in 0..cases {
            let mut bad = original.clone();
            match case {
                0 => {
                    f.cache.0.remove(&f.vaults[0]);
                }
                1 => bad.owner = Pubkey::new_unique(),
                2..=4 => {
                    let mut token = TokenAccount::unpack(&bad.data[..TokenAccount::LEN]).unwrap();
                    match case {
                        2 => token.mint = Pubkey::new_unique(),
                        3 => token.owner = Pubkey::new_unique(),
                        4 => token.state = AccountState::Uninitialized,
                        _ => unreachable!(),
                    }
                    token.pack_into_slice(&mut bad.data[..TokenAccount::LEN]);
                }
                5 => bad.data.truncate(100),
                6 => {
                    // Base (165), account type (1), extension type (2), length.
                    bad.data[168..170].copy_from_slice(&u16::MAX.to_le_bytes());
                }
                _ => unreachable!(),
            }
            if case != 0 {
                f.cache.0.insert(f.vaults[0], bad);
            }
            assert!(f.venue.update_state(&f.cache).await.is_err());
            assert!(!f.venue.initialized());
            assert!(matches!(
                f.venue.quote(request.clone()),
                Err(TradingVenueError::NotInitialized(_))
            ));
            f.cache.0.insert(f.vaults[0], original.clone());
            f.refresh().await;
            assert!(f.venue.quote(request.clone()).unwrap().expected_output > 0);
        }
    }
}

#[tokio::test]
async fn changed_token_program_cannot_reuse_old_vault_subscription() {
    let mut f = Fixture::new(spl_token::ID, false).await;
    let account = f.cache.0.get_mut(&f.venue.pool_key).unwrap();
    let mut pool = CofferPool::from_account_data(&account.data).unwrap();
    pool.tokens[0].config.token_program = spl_token_2022::ID;
    account.data = pool.to_account_data();
    f.cache.0.get_mut(&f.mints[0]).unwrap().owner = spl_token_2022::ID;
    assert!(matches!(
        f.venue.update_state(&f.cache).await,
        Err(TradingVenueError::MissingState(_))
    ));
    assert!(!f.venue.initialized());
}

#[tokio::test]
async fn failed_refresh_does_not_publish_a_partially_rebuilt_snapshot() {
    let mut f = Fixture::new(spl_token::ID, false).await;
    let original_accounts = f.cache.0.clone();
    let original_pool = *f.venue.pool();
    let request = f.request(0, 1, 1_000_000);
    let original_quote = f.venue.quote(request.clone()).unwrap();
    for case in 0..3 {
        match case {
            0 => {
                f.cache.0.remove(&clock::ID);
            }
            1 => {
                f.cache.0.remove(&f.mints[0]);
            }
            2 => {
                // This policy cannot evaluate its active window. It passes
                // account decoding, then fails while rebuilding directions.
                let mut invalid = original_pool;
                invalid.tokens[0].config.max_selloff_pct = 5_000;
                invalid.tokens[0].config.variable_fee_slope_high_pct = 2_500;
                invalid.tokens[0].config.max_selloff_period_length = 0;
                f.cache.0.get_mut(&f.venue.pool_key).unwrap().data = invalid.to_account_data();
            }
            _ => unreachable!(),
        }
        assert!(f.venue.update_state(&f.cache).await.is_err());
        assert!(!f.venue.initialized());
        assert_eq!(*f.venue.pool(), original_pool);
        assert!(matches!(
            f.venue.quote(request.clone()),
            Err(TradingVenueError::NotInitialized(_))
        ));
        f.cache.0 = original_accounts.clone();
        f.refresh().await;
        let restored = f.venue.quote(request.clone()).unwrap();
        assert_eq!(restored.expected_output, original_quote.expected_output);
        assert_eq!(restored.price, original_quote.price);
    }
}

#[tokio::test]
async fn freeze_and_thaw_match_pinned_coffer_elf() {
    // Existing fixture suite pins the hash of this committed program binary.
    const PROGRAM_SO: &str = "programs/8iQtGj9mcUfFUGaiCpPy89swC3s8YTC8FhVZWfgeZhwu.so";
    for (program, extended) in [(spl_token::ID, false), (spl_token_2022::ID, true)] {
        for frozen_slot in [0, 1] {
            let mut f = Fixture::new(program, extended).await;
            let mut svm = LiteSVM::new()
                .with_compute_budget(ComputeBudget {
                    compute_unit_limit: 1_400_000,
                    ..Default::default()
                })
                .with_blockhash_check(false)
                .with_sigverify(false)
                .with_transaction_history(0);
            svm.add_program_from_file(COFFER_PROGRAM_ID, PROGRAM_SO)
                .unwrap();
            let user = Keypair::new();
            svm.set_account(
                user.pubkey(),
                Account {
                    lamports: 10_000_000_000,
                    owner: solana_sdk::system_program::id(),
                    ..Default::default()
                },
            )
            .unwrap();
            for (key, account) in &f.cache.0 {
                if *key != clock::ID {
                    svm.set_account(*key, account.clone()).unwrap();
                }
            }
            svm.set_sysvar::<Clock>(&bincode::deserialize(&f.cache.0[&clock::ID].data).unwrap());
            for (i, mint) in f.mints.iter().enumerate() {
                let mut mint_account = svm.get_account(mint).unwrap();
                let mut data = Mint::unpack(&mint_account.data).unwrap();
                data.freeze_authority = Some(user.pubkey()).into();
                data.pack_into_slice(&mut mint_account.data);
                svm.set_account(*mint, mint_account).unwrap();
                let user_ata = CofferPool::derive_vault(&user.pubkey(), mint, &program);
                let mut account = svm.get_account(&f.vaults[i]).unwrap();
                let mut token = TokenAccount::unpack(&account.data[..TokenAccount::LEN]).unwrap();
                token.owner = user.pubkey();
                token.pack_into_slice(&mut account.data[..TokenAccount::LEN]);
                svm.set_account(user_ata, account).unwrap();
            }
            let request = f.request(0, 1, 1_000_000);
            let output_ata = CofferPool::derive_vault(&user.pubkey(), &f.mints[1], &program);
            let output_before = TokenAccount::unpack(
                &svm.get_account(&output_ata).unwrap().data[..TokenAccount::LEN],
            )
            .unwrap()
            .amount;
            let swap = f
                .venue
                .generate_swap_instruction(request.clone(), user.pubkey())
                .unwrap();
            for frozen in [true, false] {
                let toggle = if program == spl_token::ID {
                    if frozen {
                        spl_token::instruction::freeze_account
                    } else {
                        spl_token::instruction::thaw_account
                    }
                } else if frozen {
                    spl_token_2022::instruction::freeze_account
                } else {
                    spl_token_2022::instruction::thaw_account
                };
                let ix = toggle(
                    &program,
                    &f.vaults[frozen_slot],
                    &f.mints[frozen_slot],
                    &user.pubkey(),
                    &[],
                )
                .unwrap();
                svm.send_transaction(Transaction::new_signed_with_payer(
                    &[ix],
                    Some(&user.pubkey()),
                    &[&user],
                    svm.latest_blockhash(),
                ))
                .unwrap();
                let keys = f.venue.get_required_pubkeys_for_update().unwrap();
                f.cache = Snapshot(
                    keys.iter()
                        .map(|key| (*key, svm.get_account(key).unwrap()))
                        .collect(),
                );
                f.refresh().await;
                let quoted = f.venue.quote(request.clone());
                let executed = svm.send_transaction(Transaction::new_signed_with_payer(
                    std::slice::from_ref(&swap),
                    Some(&user.pubkey()),
                    &[&user],
                    svm.latest_blockhash(),
                ));
                if frozen {
                    assert!(matches!(
                        quoted,
                        Err(TradingVenueError::AmmMethodError(ErrorInfo::StaticStr(
                            "AccountFrozen"
                        )))
                    ));
                    let failure = executed.unwrap_err();
                    assert_eq!(
                        failure.err,
                        TransactionError::InstructionError(0, InstructionError::Custom(17)),
                        "{}",
                        failure.meta.logs.join("\n")
                    );
                    assert_eq!(
                        svm.get_account(&f.venue.pool_key).unwrap().data,
                        f.cache.0[&f.venue.pool_key].data,
                        "a frozen vault swap must not commit pool state"
                    );
                } else {
                    let quote = quoted.unwrap();
                    executed.unwrap();
                    let output_after = TokenAccount::unpack(
                        &svm.get_account(&output_ata).unwrap().data[..TokenAccount::LEN],
                    )
                    .unwrap()
                    .amount;
                    assert_eq!(output_after - output_before, quote.expected_output);
                    let mut predicted = *f.venue.pool();
                    let outcome =
                        quote_exact_in(&predicted, request.amount, 0, 1, 9, 9, f.venue.now())
                            .unwrap();
                    apply_swap(&mut predicted, 0, 1, &outcome).unwrap();
                    assert_eq!(
                        svm.get_account(&f.venue.pool_key).unwrap().data,
                        predicted.to_account_data()
                    );
                }
            }
        }
    }
}

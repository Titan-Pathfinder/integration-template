// Construction tests for Dr. Fraudsworth TradingVenue integration.

use std::collections::HashMap;
use async_trait::async_trait;
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;
use titan_integration_template::account_caching::{AccountCacheError, AccountsCache};
use titan_integration_template::trading_venue::{
    FromAccount, QuoteRequest, SwapType, TradingVenue,
};
use titan_integration_template::trading_venue::error::TradingVenueError;
use titan_integration_template::drfraudsworth::accounts::addresses::*;
use titan_integration_template::drfraudsworth::constants::*;
use titan_integration_template::drfraudsworth::sol_pool_venue::SolPoolVenue;
use titan_integration_template::drfraudsworth::vault_venue::{
    VaultVenue, known_sol_pool_venues, known_vault_venues,
};

struct MockCache { accounts: HashMap<Pubkey, Account> }
impl MockCache {
    fn with_pool_and_epoch(pool_key: Pubkey, pool_data: Vec<u8>, epoch_data: Vec<u8>) -> Self {
        let mut accounts = HashMap::new();
        accounts.insert(pool_key, Account { lamports: 1_000_000, data: pool_data, owner: AMM_PROGRAM_ID, executable: false, rent_epoch: 0 });
        accounts.insert(EPOCH_STATE_PDA, Account { lamports: 1_000_000, data: epoch_data, owner: EPOCH_PROGRAM_ID, executable: false, rent_epoch: 0 });
        Self { accounts }
    }
    fn with_vault_config() -> Self {
        let mut accounts = HashMap::new();
        accounts.insert(VAULT_CONFIG_PDA, Account { lamports: 1_000_000, data: vec![0u8; 64], owner: CONVERSION_VAULT_PROGRAM_ID, executable: false, rent_epoch: 0 });
        Self { accounts }
    }
}
#[async_trait]
impl AccountsCache for MockCache {
    async fn get_account(&self, pubkey: &Pubkey) -> Result<Option<Account>, AccountCacheError> { Ok(self.accounts.get(pubkey).cloned()) }
    async fn get_accounts(&self, pubkeys: &[Pubkey]) -> Result<Vec<Option<Account>>, AccountCacheError> { Ok(pubkeys.iter().map(|pk| self.accounts.get(pk).cloned()).collect()) }
}

fn mock_pool_bytes(mint_a: &Pubkey, ra: u64, rb: u64, fee: u16) -> Vec<u8> {
    let mut d = vec![0u8; 224]; d[9..41].copy_from_slice(mint_a.as_ref());
    d[137..145].copy_from_slice(&ra.to_le_bytes()); d[145..153].copy_from_slice(&rb.to_le_bytes());
    d[153..155].copy_from_slice(&fee.to_le_bytes()); d
}
fn mock_epoch_bytes(cb: u16, cs: u16, fb: u16, fs: u16) -> Vec<u8> {
    let mut d = vec![0u8; 172]; d[0..8].copy_from_slice(&EPOCH_STATE_DISCRIMINATOR);
    d[33..35].copy_from_slice(&cb.to_le_bytes()); d[35..37].copy_from_slice(&cs.to_le_bytes());
    d[37..39].copy_from_slice(&fb.to_le_bytes()); d[39..41].copy_from_slice(&fs.to_le_bytes()); d
}

#[test]
fn from_account_parses_pool() {
    let acct = Account { lamports: 1_000_000, data: mock_pool_bytes(&NATIVE_MINT, 100_000_000_000, 500_000_000_000, 100), owner: AMM_PROGRAM_ID, executable: false, rent_epoch: 0 };
    let venue = SolPoolVenue::from_account(&CRIME_SOL_POOL, &acct).unwrap();
    assert_eq!(venue.market_id(), CRIME_SOL_POOL);
}

#[tokio::test]
async fn update_state_loads_reserves() {
    let cache = MockCache::with_pool_and_epoch(CRIME_SOL_POOL, mock_pool_bytes(&NATIVE_MINT, 100_000_000_000, 500_000_000_000, 100), mock_epoch_bytes(400, 1400, 1400, 400));
    let mut venue = SolPoolVenue::new_uninitialized(true, CRIME_SOL_POOL, CRIME_MINT);
    venue.update_state(&cache).await.unwrap();
    assert!(venue.initialized());
    let r = venue.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: 1_000_000_000, swap_type: SwapType::ExactIn }).unwrap();
    assert!(r.expected_output > 0);
}

#[tokio::test]
async fn vault_update_state() {
    let cache = MockCache::with_vault_config();
    let mut v = known_vault_venues()[0].clone();
    v.update_state(&cache).await.unwrap();
    assert!(v.initialized());
}

#[test]
fn no_transfer_fees() {
    for v in known_sol_pool_venues() { for t in v.get_token_info() { assert!(t.transfer_fee.is_none()); } }
    for v in known_vault_venues() { for t in v.get_token_info() { assert!(t.transfer_fee.is_none()); } }
}

#[test]
fn bounds_exist() {
    let v = SolPoolVenue::new_for_testing(true, 100_000_000_000, 1_000_000_000_000, 400, 1400);
    assert!(v.bounds(0, 1).is_ok());
    assert!(v.bounds(1, 0).is_ok());
    let vv = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
    let (lo, _) = vv.bounds(0, 1).unwrap();
    assert!(lo >= 100);
}

#[test]
fn zero_input_no_panic() {
    let v = SolPoolVenue::new_for_testing(true, 100_000_000_000, 1_000_000_000_000, 400, 1400);
    assert_eq!(v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: 0, swap_type: SwapType::ExactIn }).unwrap().expected_output, 0);
    let vv = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
    assert_eq!(vv.quote(QuoteRequest { input_mint: CRIME_MINT, output_mint: PROFIT_MINT, amount: 0, swap_type: SwapType::ExactIn }).unwrap().expected_output, 0);
}

#[test]
fn exact_out_rejected() {
    let v = SolPoolVenue::new_for_testing(true, 100_000_000_000, 1_000_000_000_000, 400, 1400);
    assert!(matches!(v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: 1_000_000_000, swap_type: SwapType::ExactOut }), Err(TradingVenueError::ExactOutNotSupported)));
}

#[test]
fn speed_under_100us() {
    let v = SolPoolVenue::new_for_testing(true, 100_000_000_000, 1_000_000_000_000, 400, 1400);
    let start = std::time::Instant::now();
    for i in 0..10_000u64 { let _ = v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: 1_000_000 + i, swap_type: SwapType::ExactIn }); }
    assert!(start.elapsed().as_micros() as f64 / 10_000.0 < 100.0);
}

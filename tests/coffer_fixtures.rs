//! Offline LiteSVM fixture suite for the Coffer venue.
//!
//! No mainnet pool has a sell-off cap with an aggressive surge curve, a
//! deactivated token or a disabled switch, so this suite builds synthetic pools
//! (the real account layout, real PDA seeds, real ATAs and mints) and runs
//! the PRODUCTION Coffer ELF (`programs/8iQt….so`, sha256-pinned below)
//! against them in LiteSVM. Executed swaps assert that the user's on-chain
//! `amount_out` equals the raw contract port exactly, and that the program's
//! post-swap pool account equals the port's `apply_swap` prediction byte for
//! byte (window rotation, snapshot capture, carry-over rescale, protocol-fee
//! buckets included). Adapter quotes are checked inside its supported domain;
//! raw surge parity remains covered beyond that domain.
//!
//! It needs no RPC — only the committed program binary — so it always runs.

#![allow(clippy::result_large_err)] // `TradingVenueError` is large (crate-wide allow in the lib too)

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use async_trait::async_trait;
use litesvm::LiteSVM;
use solana_account::{Account, WritableAccount};
use solana_compute_budget::compute_budget::ComputeBudget;
use solana_instruction::error::InstructionError;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::TransactionError;
use solana_sysvar::clock::Clock;
use solana_transaction::Transaction;
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

use titan_integration_template::account_caching::{AccountCacheError, AccountsCache};
use titan_integration_template::coffer::errors::ErrorCode;
use titan_integration_template::coffer::state::{AssetConfig, CofferPool};
use titan_integration_template::coffer::swap::{SwapOutcome, apply_swap, quote_exact_in};
use titan_integration_template::coffer_venue::{COFFER_PROGRAM_ID, CofferVenue};
use titan_integration_template::trading_venue::error::TradingVenueError;
use titan_integration_template::trading_venue::{
    FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
};

/// The production Coffer bytecode (byte-identical to mainnet).
const PROGRAM_SO: &str = "programs/8iQtGj9mcUfFUGaiCpPy89swC3s8YTC8FhVZWfgeZhwu.so";
const PROGRAM_SHA256: &str = "5128c57884b910cdc8dfe2cf96db4e6acb7ef5efe915f8912076b37dccd5afb7";

const NOW: i64 = 1_800_000_000;
const PERIOD: u32 = 3_600;

// ---------------------------------------------------------------------------
// In-memory account cache: a snapshot of LiteSVM accounts.
// ---------------------------------------------------------------------------

struct SvmCache(HashMap<Pubkey, Account>);

impl SvmCache {
    fn snapshot(svm: &LiteSVM, keys: &[Pubkey]) -> Self {
        Self(
            keys.iter()
                .filter_map(|k| svm.get_account(k).map(|a| (*k, a)))
                .collect(),
        )
    }
}

#[async_trait]
impl AccountsCache for SvmCache {
    async fn get_account(&self, pubkey: &Pubkey) -> Result<Option<Account>, AccountCacheError> {
        Ok(self.0.get(pubkey).cloned())
    }
    async fn get_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<Account>>, AccountCacheError> {
        Ok(pubkeys.iter().map(|k| self.0.get(k).cloned()).collect())
    }
}

// ---------------------------------------------------------------------------
// Fixture construction
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct TokenSpec {
    decimals: u8,
    weight: u64,
    virtual_balance: u64,
    actual_balance: u64,
    protocol_fees_owed: u64,
    token_2022: bool,
    config: AssetConfig,
}

impl TokenSpec {
    fn new(decimals: u8, weight: u64, virtual_balance: u64, actual_balance: u64) -> Self {
        Self {
            decimals,
            weight,
            virtual_balance,
            actual_balance,
            protocol_fees_owed: 0,
            token_2022: false,
            config: AssetConfig::default(),
        }
    }
    fn t22(mut self) -> Self {
        self.token_2022 = true;
        self
    }
    fn cap(mut self, max_selloff_pct: u16) -> Self {
        self.config.max_selloff_pct = max_selloff_pct;
        self.config.max_selloff_period_length = PERIOD;
        self
    }
    fn surge(mut self, threshold: u16, low: u16, mid: u16, high: u16, kink: u8) -> Self {
        self.config.variable_fee_threshold_pct = threshold;
        self.config.variable_fee_slope_low_pct = low;
        self.config.variable_fee_slope_mid_pct = mid;
        self.config.variable_fee_slope_high_pct = high;
        self.config.variable_fee_kink_pct = kink;
        self
    }
    fn inactive(mut self) -> Self {
        self.config.is_active = false;
        self
    }
}

struct Fixture {
    svm: LiteSVM,
    user: Keypair,
    pool_key: Pubkey,
    mints: Vec<Pubkey>,
    token_programs: Vec<Pubkey>,
    venue: CofferVenue,
}

fn token_program(t22: bool) -> Pubkey {
    if t22 {
        spl_token_2022::ID
    } else {
        spl_token::ID
    }
}

fn mint_account(decimals: u8, owner: Pubkey) -> Account {
    let mut account = Account::new(LAMPORTS_PER_SOL, Mint::LEN, &owner);
    Mint {
        mint_authority: None.into(),
        supply: u64::MAX / 2,
        decimals,
        is_initialized: true,
        freeze_authority: None.into(),
    }
    .pack_into_slice(account.data_as_mut_slice());
    account
}

fn token_account(mint: Pubkey, owner: Pubkey, amount: u64, program: Pubkey) -> Account {
    let mut account = Account::new(LAMPORTS_PER_SOL, TokenAccount::LEN, &program);
    TokenAccount {
        mint,
        owner,
        amount,
        state: AccountState::Initialized,
        ..Default::default()
    }
    .pack_into_slice(account.data_as_mut_slice());
    account
}

fn ata(owner: &Pubkey, mint: &Pubkey, program: &Pubkey) -> Pubkey {
    spl_associated_token_account::get_associated_token_address_with_program_id(owner, mint, program)
}

fn build_pool(
    config: Pubkey,
    pool_id: u64,
    swap_fee_rate: u32,
    tokens: &[TokenSpec],
) -> (Pubkey, CofferPool) {
    let (pool_key, bump) = CofferPool::derive_pool_address(&config, pool_id, &COFFER_PROGRAM_ID);
    let mut pool = CofferPool {
        config,
        bump,
        token_count: tokens.len() as u8,
        pool_id,
        swap_fee_rate,
        protocol_fee_rate: 2_000,
        created_at: NOW - 1_000,
        pool_admin: Pubkey::new_unique(),
        ..Default::default()
    };
    for (i, t) in tokens.iter().enumerate() {
        let slot = &mut pool.tokens[i];
        slot.config = t.config;
        slot.config.mint = Pubkey::new_unique();
        slot.config.token_program = token_program(t.token_2022);
        slot.config.normalized_weight = t.weight;
        slot.dynamics.virtual_balance = t.virtual_balance;
        slot.dynamics.actual_balance = t.actual_balance;
        slot.dynamics.protocol_fees_owed = t.protocol_fees_owed;
        slot.dynamics.window_start_timestamp = NOW - 1_000;
    }
    (pool_key, pool)
}

impl Fixture {
    fn new(swap_fee_rate: u32, tokens: &[TokenSpec]) -> Self {
        Self::with_pool(swap_fee_rate, tokens, |_| {})
    }

    /// Build the fixture, letting `patch` edit the pool bytes-to-be before
    /// they are written (e.g. to disable the pool).
    fn with_pool(
        swap_fee_rate: u32,
        tokens: &[TokenSpec],
        patch: impl FnOnce(&mut CofferPool),
    ) -> Self {
        assert!(Path::new(PROGRAM_SO).exists(), "missing {PROGRAM_SO}");
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
                lamports: 10_000 * LAMPORTS_PER_SOL,
                data: vec![],
                owner: solana_sdk::system_program::id(),
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

        let (pool_key, mut pool) = build_pool(Pubkey::new_unique(), 7, swap_fee_rate, tokens);
        patch(&mut pool);

        let mut mints = Vec::new();
        let mut token_programs = Vec::new();
        for (i, t) in tokens.iter().enumerate() {
            let cfg = pool.tokens[i].config;
            let program = cfg.token_program;
            svm.set_account(cfg.mint, mint_account(t.decimals, program))
                .unwrap();
            // vault = actual + protocol fees owed
            let vault = ata(&pool_key, &cfg.mint, &program);
            svm.set_account(
                vault,
                token_account(
                    cfg.mint,
                    pool_key,
                    t.actual_balance + t.protocol_fees_owed,
                    program,
                ),
            )
            .unwrap();
            // User ATAs, funded so the u64 balance ceiling is the POOL's, not
            // the user's: the pool can absorb at most `u64::MAX - vb` of a token
            // and pay out at most `actual_balance`, so `u64::MAX - vault` on the
            // user side never overflows in either role (vb >= vault here).
            svm.set_account(
                ata(&user.pubkey(), &cfg.mint, &program),
                token_account(
                    cfg.mint,
                    user.pubkey(),
                    u64::MAX - t.actual_balance - t.protocol_fees_owed,
                    program,
                ),
            )
            .unwrap();
            mints.push(cfg.mint);
            token_programs.push(program);
        }

        let mut pool_account = Account::new(LAMPORTS_PER_SOL, CofferPool::LEN, &COFFER_PROGRAM_ID);
        pool_account.data = pool.to_account_data();
        svm.set_account(pool_key, pool_account).unwrap();

        let venue =
            CofferVenue::from_account(&pool_key, &svm.get_account(&pool_key).unwrap()).unwrap();
        let mut fx = Self {
            svm,
            user,
            pool_key,
            mints,
            token_programs,
            venue,
        };
        fx.set_clock(NOW);
        fx.refresh();
        fx
    }

    fn set_clock(&mut self, unix_timestamp: i64) {
        self.svm.set_sysvar::<Clock>(&Clock {
            unix_timestamp,
            ..Default::default()
        });
    }

    fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    /// Re-read the pool, mints and clock from the SVM into the venue — what
    /// Titan does between swaps.
    fn refresh(&mut self) {
        let keys = self.venue.get_required_pubkeys_for_update().unwrap();
        let cache = SvmCache::snapshot(&self.svm, &keys);
        futures_block_on(self.venue.update_state(&cache)).unwrap();
        assert!(self.venue.initialized());
        assert_eq!(self.venue.now(), self.now());
    }

    fn pool(&self) -> CofferPool {
        CofferPool::from_account_data(&self.svm.get_account(&self.pool_key).unwrap().data).unwrap()
    }

    fn request(&self, i: usize, j: usize, amount: u64) -> QuoteRequest {
        QuoteRequest {
            input_mint: self.mints[i],
            output_mint: self.mints[j],
            amount,
            swap_type: SwapType::ExactIn,
        }
    }

    fn quote(&self, i: usize, j: usize, amount: u64) -> Result<QuoteResult, TradingVenueError> {
        self.venue.quote(self.request(i, j, amount))
    }

    fn user_balance(&self, i: usize) -> u64 {
        let key = ata(&self.user.pubkey(), &self.mints[i], &self.token_programs[i]);
        TokenAccount::unpack_from_slice(&self.svm.get_account(&key).unwrap().data)
            .unwrap()
            .amount
    }

    /// Execute a swap through the venue's instruction. Returns the user's
    /// received output atoms, or the program's custom error code.
    fn swap(&mut self, i: usize, j: usize, amount: u64) -> Result<u64, u32> {
        let ix = self
            .venue
            .generate_swap_instruction(self.request(i, j, amount), self.user.pubkey())
            .unwrap();
        let before = self.user_balance(j);
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.user.pubkey()),
            &[&self.user],
            self.svm.latest_blockhash(),
        );
        match self.svm.send_transaction(tx) {
            Ok(_) => Ok(self.user_balance(j) - before),
            Err(failed) => match failed.err {
                TransactionError::InstructionError(_, InstructionError::Custom(code)) => Err(code),
                other => panic!(
                    "unexpected transaction failure: {other:?}\n{}",
                    failed.meta.logs.join("\n")
                ),
            },
        }
    }

    /// `n` geometrically spaced sizes across the CURRENT valid range of a
    /// direction, executing each (the range moves as the pool state does, so
    /// the bounds are recomputed before every sample).
    fn sweep(&mut self, i: usize, j: usize, n: usize) {
        for k in 0..n {
            let (lb, ub) = self.venue.bounds(i as u8, j as u8).unwrap();
            // The pool's u64 ceiling can exceed what the fixture user holds
            // (earlier samples paid protocol fees into the pool).
            let ub = ub.min(self.user_balance(i));
            assert!(lb < ub, "{i}->{j}: empty range");
            let t = k as f64 / (n - 1) as f64;
            let amount = (((lb as f64).ln() + t * ((ub as f64).ln() - (lb as f64).ln())).exp()
                as u64)
                .clamp(lb, ub);
            self.check_swap(i, j, amount);
        }
    }

    /// The full parity check for one swap: quote, predict the post-state,
    /// execute, compare the payout AND the whole pool account, then refresh.
    fn check_swap(&mut self, i: usize, j: usize, amount: u64) -> QuoteResult {
        let quote = self.quote(i, j, amount).unwrap();
        assert!(
            !quote.not_enough_liquidity,
            "{i}->{j} {amount}: unexpected partial fill {quote:?}"
        );
        assert_eq!(quote.amount, amount);

        let outcome = self.check_raw_swap(i, j, amount);
        assert_eq!(outcome.amount_out_user, quote.expected_output);
        quote
    }

    /// Verify the complete contract port independently of Titan's supported
    /// domain. Surge swaps outside that domain must retain exact ELF parity.
    fn check_raw_swap(&mut self, i: usize, j: usize, amount: u64) -> SwapOutcome {
        let pool_before = self.pool();
        let outcome = quote_exact_in(
            &pool_before,
            amount,
            i as u8,
            j as u8,
            self.venue.get_token(i).unwrap().decimals as u8,
            self.venue.get_token(j).unwrap().decimals as u8,
            self.now(),
        )
        .unwrap();
        let mut predicted = pool_before;
        apply_swap(&mut predicted, i as u8, j as u8, &outcome).unwrap();
        let received = self
            .swap(i, j, amount)
            .unwrap_or_else(|code| panic!("{i}->{j} amount {amount}: program failed with {code}"));
        assert_eq!(
            received, outcome.amount_out_user,
            "{i}->{j} amount {amount}: on-chain != raw port"
        );
        assert_eq!(
            self.pool(),
            predicted,
            "{i}->{j} amount {amount}: post-swap pool state differs"
        );
        self.refresh();
        outcome
    }

    /// Inject an account update, then refresh the SAME venue. These snapshots
    /// model RM/admin writes; the local-validator suite executes the real
    /// management instructions separately.
    fn update_pool(&mut self, patch: impl FnOnce(&mut CofferPool)) {
        let mut pool = self.pool();
        patch(&mut pool);
        let mut account = self.svm.get_account(&self.pool_key).unwrap();
        account.data = pool.to_account_data();
        self.svm.set_account(self.pool_key, account).unwrap();
        self.refresh();
    }
}

fn futures_block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

/// Log-spaced sizes in `[lo, hi]` plus both ends.
fn grid(lo: u64, hi: u64, n: usize) -> Vec<u64> {
    let mut out: Vec<u64> = (0..n)
        .map(|k| {
            let t = k as f64 / (n - 1) as f64;
            ((lo as f64).ln() + t * ((hi as f64).ln() - (lo as f64).ln())).exp() as u64
        })
        .map(|x| x.clamp(lo, hi))
        .collect();
    out.push(lo);
    out.push(hi);
    out.sort();
    out.dedup();
    out
}

fn sha256_hex(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap();
    let digest = solana_program::hash::hash(&bytes);
    digest
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn program_binary_is_the_pinned_production_build() {
    assert_eq!(
        sha256_hex(PROGRAM_SO),
        PROGRAM_SHA256,
        "{PROGRAM_SO} is not the pinned mainnet build"
    );
}

/// Plain pool, both directions, a log grid of sizes: exact payout and exact
/// post-state (the LP fee / protocol fee split included).
#[test]
fn plain_pool_grid_matches_onchain_exactly() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(6, 5_000, 5_000_000_000_000, 5_000_000_000_000),
            TokenSpec::new(9, 5_000, 25_000_000_000_000, 25_000_000_000_000),
        ],
    );
    for (i, j) in [(0usize, 1usize), (1, 0)] {
        fx.sweep(i, j, 12);
    }
}

/// The top of a direction's range is the pool's u64 balance ceiling: the
/// program's checked `virtual_balance` update overflows one atom past it, and
/// so does the quote (`MathError`), at exactly the same input.
#[test]
fn u64_balance_ceiling_is_exact() {
    let room = 1_000_000u64;
    let mut fx = Fixture::new(
        0,
        &[
            TokenSpec::new(6, 5_000, u64::MAX - room, 10_000_000),
            // Comparable virtual depth on the other side so `room` atoms of
            // input still produce output (the ceiling, not rounding, binds).
            TokenSpec::new(6, 5_000, u64::MAX / 2, 1_000_000_000_000),
        ],
    );
    assert!(!fx.quote(0, 1, room).unwrap().not_enough_liquidity);
    let over = fx.quote(0, 1, room + 1);
    assert!(
        matches!(over, Err(TradingVenueError::MathError(_))),
        "{over:?}"
    );
    let (_, ub) = fx.venue.bounds(0, 1).unwrap();
    assert!(ub <= room && room - ub <= 100, "ub {ub} vs ceiling {room}");
    assert_eq!(fx.swap(0, 1, room + 1), Err(ErrorCode::MathOverflow.code()));
    fx.refresh();
    fx.check_swap(0, 1, room);
    assert_eq!(fx.pool().tokens[0].dynamics.virtual_balance, u64::MAX);
}

/// The sell-off cap: the quote's partial-fill `amount` is EXACTLY the largest
/// input the program accepts; one atom more reverts with `MaxSelloffExceeded`.
#[test]
fn max_selloff_boundary_is_exact() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(9, 5_000, 10_000_000_000_000, 10_000_000_000_000).cap(1_000),
            TokenSpec::new(6, 5_000, 10_000_000_000, 10_000_000_000),
        ],
    );
    let cap = 10_000_000_000_000 / 10;
    let q = fx.quote(0, 1, u64::MAX / 8).unwrap();
    assert!(q.not_enough_liquidity);
    assert_eq!(
        q.amount, cap,
        "headroom must be the whole cap on a fresh window"
    );
    assert!(q.expected_output > 0 && q.price > 0.0);

    // Titan's boundary search lands on (within 100 atoms of) the same edge.
    let (_, ub) = fx.venue.bounds(0, 1).unwrap();
    assert!(
        ub <= cap && cap - ub <= 100,
        "upper bound {ub} vs headroom {cap}"
    );

    // One atom past the headroom reverts on-chain with the same error the
    // port predicts, and the pool is untouched.
    let before = fx.pool();
    assert_eq!(
        fx.swap(0, 1, cap + 1),
        Err(ErrorCode::MaxSelloffExceeded.code())
    );
    assert_eq!(fx.pool(), before);
    fx.refresh();

    // Exactly the headroom succeeds, with the quoted output.
    let q = fx.check_swap(0, 1, cap);
    assert_eq!(q.amount, cap);
    assert_eq!(fx.pool().tokens[0].dynamics.current_selloff, cap);
    assert_eq!(
        fx.pool().tokens[0].dynamics.selloff_vb_snapshot,
        10_000_000_000_000
    );

    // Window now full: headroom 0 → any sell is a zero partial fill, and the
    // direction stays declared (the window rotates on its own).
    let q = fx.quote(0, 1, 1).unwrap();
    assert!(q.not_enough_liquidity && q.amount == 0 && q.expected_output == 0);
    assert!(fx.venue.directions_num().contains(&(0, 1)));
    assert!(fx.venue.bounds(0, 1).is_err());
    assert_eq!(fx.swap(0, 1, 1), Err(ErrorCode::MaxSelloffExceeded.code()));
    // The other direction is uncapped and unaffected.
    fx.refresh();
    fx.check_swap(1, 0, 1_000_000);
}

/// The end of the last untaxed fill-ratio cell is included exactly. Every
/// larger request returns that same executable amount, including requests
/// beyond the contract's hard sell-off cap.
#[test]
fn partial_fill_stops_at_exact_zero_surge_boundary() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(9, 5_000, 10_000_000_000_000, 10_000_000_000_000)
                .cap(1_000)
                .surge(6_000, 1_000, 5_000, 10_000, 80),
            TokenSpec::new(6, 5_000, 10_000_000_000, 10_000_000_000),
        ],
    );
    let cap = 1_000_000_000_000u64;
    let limit = 600_099_999_999u64;
    assert_eq!(fx.venue.fill_limit(0, 1), Some(limit));
    let at_limit = fx.quote(0, 1, limit).unwrap();
    assert!(!at_limit.not_enough_liquidity);
    for amount in [limit - 1, limit] {
        let q = fx.quote(0, 1, amount).unwrap();
        assert_eq!(q.amount, amount);
        assert!(!q.not_enough_liquidity);
        let raw = quote_exact_in(&fx.pool(), amount, 0, 1, 9, 6, fx.now()).unwrap();
        assert_eq!(raw.surge_fee_amount, 0);
        assert_eq!(q.expected_output, raw.amount_out_user);
    }
    assert!(
        quote_exact_in(&fx.pool(), limit + 1, 0, 1, 9, 6, fx.now())
            .unwrap()
            .surge_fee_amount
            > 0
    );
    for requested in [limit + 1, cap, cap + 1, u64::MAX / 8] {
        let q = fx.quote(0, 1, requested).unwrap();
        assert!(q.not_enough_liquidity);
        assert_eq!(q.amount, limit);
        assert_eq!(q.expected_output, at_limit.expected_output);
        assert_eq!(q.price, at_limit.price);
    }
    let (_, upper) = fx.venue.bounds(0, 1).unwrap();
    assert!(upper <= limit && limit - upper <= 100);
    fx.check_swap(0, 1, limit);
    let exhausted = fx.quote(0, 1, 1).unwrap();
    assert!(exhausted.not_enough_liquidity);
    assert_eq!((exhausted.amount, exhausted.expected_output), (0, 0));
    assert!(fx.venue.bounds(0, 1).is_err());
    assert!(fx.venue.directions_num().contains(&(0, 1)));
}

/// Split versus whole: two sequential sells (re-reading the pool and calling
/// `update_state` between them) each match on-chain, and the accumulated
/// window state matches the prediction.
#[test]
fn split_sequence_matches_onchain_with_state_refresh() {
    let mut fx = Fixture::new(
        5_000,
        &[
            TokenSpec::new(9, 6_000, 10_000_000_000_000, 8_000_000_000_000)
                .cap(2_000)
                .surge(7_000, 50, 500, 2_500, 85),
            TokenSpec::new(6, 4_000, 40_000_000_000, 30_000_000_000),
        ],
    );
    let cap = 10_000_000_000_000 / 5;
    let parts = [cap * 3 / 10, cap * 3 / 10, cap * 3 / 10];
    let mut headroom = cap;
    for part in parts {
        let q = fx.check_raw_swap(0, 1, part);
        headroom -= part;
        assert_eq!(
            titan_integration_template::coffer::swap::selloff_headroom(&fx.pool(), 0, fx.now())
                .unwrap(),
            Some(headroom),
            "headroom after {part}"
        );
        eprintln!(
            "part {part}: out {} surge in state {}",
            q.amount_out_user,
            fx.pool().tokens[1].dynamics.protocol_fees_owed
        );
    }
    // Last part crossed the 70% threshold: the surge fee landed in the
    // OUTPUT token's protocol bucket.
    assert!(fx.pool().tokens[1].dynamics.protocol_fees_owed > 0);
    // Remaining headroom executes exactly, then the window is full.
    fx.check_raw_swap(0, 1, headroom);
    assert_eq!(fx.swap(0, 1, 1), Err(ErrorCode::MaxSelloffExceeded.code()));
}

/// Warping the clock past one and two periods: the port rotates the window,
/// weights the carry-over and re-captures the snapshot exactly like the
/// program, including the carry-over RESCALE when the snapshot changes.
#[test]
fn window_rotation_and_carryover_rescale() {
    let mut fx = Fixture::new(
        1_000,
        &[
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000).cap(3_000),
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000),
        ],
    );
    let vb0 = 1_000_000_000_000u64;
    let cap0 = vb0 * 3 / 10;
    // Fill 60% of the window at t0.
    fx.check_swap(0, 1, cap0 * 6 / 10);
    let d = fx.pool().tokens[0].dynamics;
    assert_eq!(d.selloff_vb_snapshot, vb0);
    assert_eq!(d.current_selloff, cap0 * 6 / 10);
    // The sells raised vb_in, so the next window's snapshot will differ.
    let vb1 = d.virtual_balance;
    assert!(vb1 > vb0);

    // One period + 10s after the window OPENED (it opened at NOW - 1_000):
    // single rotation, carry-over weighted by the remaining fraction AND
    // rescaled by vb1/vb0.
    let opened = NOW - 1_000;
    fx.set_clock(opened + PERIOD as i64 + 10);
    fx.refresh();
    let probe = fx.quote(0, 1, u64::MAX / 8).unwrap();
    let rescaled_prev = (cap0 * 6 / 10) as u128 * vb1 as u128 / vb0 as u128;
    let weighted = rescaled_prev * (PERIOD as u128 - 10) / PERIOD as u128;
    let cap1 = vb1 * 3 / 10;
    assert_eq!(
        probe.amount as u128,
        cap1 as u128 - weighted,
        "headroom after one rotation"
    );
    let q = fx.check_swap(0, 1, probe.amount / 2);
    assert!(q.expected_output > 0);
    let d = fx.pool().tokens[0].dynamics;
    assert_eq!(d.window_start_timestamp, opened + PERIOD as i64);
    assert_eq!(d.previous_selloff as u128, rescaled_prev);
    assert_eq!(d.selloff_vb_snapshot, vb1);

    // Two periods later: everything cleared, fresh snapshot, full cap.
    let vb2 = d.virtual_balance;
    let later = opened + 3 * PERIOD as i64 + 5;
    fx.set_clock(later);
    fx.refresh();
    let probe = fx.quote(0, 1, u64::MAX / 8).unwrap();
    assert_eq!(probe.amount, vb2 * 3 / 10);
    fx.check_swap(0, 1, 123_456_789);
    let d = fx.pool().tokens[0].dynamics;
    assert_eq!(d.previous_selloff, 0);
    assert_eq!(d.current_selloff, 123_456_789);
    assert_eq!(d.window_start_timestamp, later);
    assert_eq!(d.selloff_vb_snapshot, vb2);
}

/// Surge fee across several curve shapes, including a near-100% threshold,
/// no kink, threshold zero, and an 20/80 weight pool (where the segmented
/// charge matters most). Every size on a grid crossing the threshold matches
/// on-chain exactly, surge fee included.
#[test]
fn surge_fee_shapes_match_onchain_exactly() {
    type Shape = (&'static str, (u16, u16, u16, u16, u8), (u64, u64));
    let shapes: [Shape; 5] = [
        ("kinked 80%", (8_000, 100, 1_000, 3_000, 90), (5_000, 5_000)),
        ("no kink", (6_000, 0, 0, 2_000, 0), (5_000, 5_000)),
        ("threshold 0", (0, 0, 50, 5_000, 50), (5_000, 5_000)),
        (
            "near-100% threshold",
            (9_990, 500, 500, 10_000, 0),
            (5_000, 5_000),
        ),
        (
            "20/80 weights kinked",
            (7_500, 10, 800, 4_000, 95),
            (2_000, 8_000),
        ),
    ];
    for (name, (thr, lo, mid, hi, kink), (w_in, w_out)) in shapes {
        let mut fx = Fixture::new(
            2_500,
            &[
                TokenSpec::new(6, w_in, 2_000_000_000_000, 2_000_000_000_000)
                    .cap(5_000)
                    .surge(thr, lo, mid, hi, kink),
                TokenSpec::new(9, w_out, 8_000_000_000_000, 8_000_000_000_000),
            ],
        );
        let cap = 2_000_000_000_000 / 2;
        let mut surged = 0;
        // Sizes from 1 atom up to the whole cap (crossing the threshold), then
        // the exact boundary.
        for amount in grid(1, cap, 24) {
            let outcome = fx.check_raw_swap(0, 1, amount);
            if outcome.surge_fee_amount > 0 {
                surged += 1;
                assert!(outcome.amount_out_user < outcome.amount_out);
            }
            let vb_snapshot = fx.pool().tokens[0].dynamics.selloff_vb_snapshot;
            assert_eq!(
                vb_snapshot, 2_000_000_000_000,
                "{name}: snapshot captured on first swap"
            );
            fx = Fixture::new(
                2_500,
                &[
                    TokenSpec::new(6, w_in, 2_000_000_000_000, 2_000_000_000_000)
                        .cap(5_000)
                        .surge(thr, lo, mid, hi, kink),
                    TokenSpec::new(9, w_out, 8_000_000_000_000, 8_000_000_000_000),
                ],
            );
        }
        assert!(surged > 0, "{name}: no sample crossed into the surge zone");
        eprintln!("{name}: {surged} surged samples all matched");
    }
}

/// Surge fee on a partially filled window: sizes that straddle the threshold
/// and reach the cap edge, with the split-vs-whole path.
#[test]
fn surge_fee_on_partially_filled_window() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(9, 5_000, 4_000_000_000_000, 4_000_000_000_000)
                .cap(2_500)
                .surge(6_000, 200, 1_500, 4_000, 80),
            TokenSpec::new(6, 5_000, 4_000_000_000, 4_000_000_000),
        ],
    );
    let cap = 4_000_000_000_000 / 4;
    // Bring the window to 50% (below the 60% threshold) first.
    fx.check_swap(0, 1, cap / 2);
    let (lb, ub) = fx.venue.bounds(0, 1).unwrap();
    assert_eq!(fx.venue.fill_limit(0, 1), Some(100_099_999_999));
    assert!(ub <= 100_099_999_999);
    let mut prev_out = 0;
    for amount in grid(lb, ub, 16) {
        let q = fx.quote(0, 1, amount).unwrap();
        assert!(
            q.expected_output >= prev_out,
            "output not monotone at {amount}"
        );
        prev_out = q.expected_output;
    }
    // Execute a chain of sells that crosses the threshold and ends at the cap.
    for amount in [cap / 10, cap / 10, cap / 10, cap / 10, cap / 10] {
        fx.check_raw_swap(0, 1, amount);
    }
    assert_eq!(fx.swap(0, 1, 1), Err(ErrorCode::MaxSelloffExceeded.code()));
    assert!(fx.pool().tokens[1].dynamics.protocol_fees_owed > 0);
}

/// Zero-fee pool and dust inputs on a fee pool (the ceil'd fee eats the whole
/// input; output 0 is what the program pays too).
#[test]
fn zero_fee_pool_and_dust_inputs() {
    let mut fx = Fixture::new(
        0,
        &[
            TokenSpec::new(6, 5_000, 1_000_000_000_000, 1_000_000_000_000),
            TokenSpec::new(6, 5_000, 1_000_000_000_000, 1_000_000_000_000),
        ],
    );
    for amount in [1u64, 2, 3, 1_000, 999_999_999] {
        let q = fx.check_swap(0, 1, amount);
        assert!(q.expected_output <= amount);
        assert_eq!(fx.pool().tokens[0].dynamics.protocol_fees_owed, 0);
    }

    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(6, 5_000, 1_000_000_000_000, 1_000_000_000_000),
            TokenSpec::new(9, 5_000, 1_000_000_000_000_000, 1_000_000_000_000_000),
        ],
    );
    // amount 1: fee ceil(0.003) = 1 → nothing reaches the curve → output 0.
    let q = fx.quote(0, 1, 1).unwrap();
    assert_eq!(q.expected_output, 0);
    assert!(!q.not_enough_liquidity);
    assert_eq!(fx.swap(0, 1, 1), Ok(0));
    assert_eq!(fx.pool().tokens[0].dynamics.protocol_fees_owed, 1);
    fx.refresh();
    for amount in [2u64, 7, 333, 334, 335, 667, 1_000, 1_001] {
        fx.check_swap(0, 1, amount);
    }
    // The boundary search skips the zero-output atom.
    let (lb, _) = fx.venue.bounds(0, 1).unwrap();
    assert!(lb >= 2);
}

/// Curve output above the LP-owned balance: the quote's partial-fill amount is
/// exactly the largest input the program accepts; one atom more reverts with
/// `AmountOutExceedsBalance`. Protocol fees sitting in the vault are NOT
/// available to swaps.
#[test]
fn output_capped_by_actual_balance() {
    let mut fx = Fixture::new(
        2_000,
        &[
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000),
            TokenSpec {
                protocol_fees_owed: 50_000_000,
                ..TokenSpec::new(6, 5_000, 1_000_000_000_000, 2_000_000)
            },
        ],
    );
    let q = fx.quote(0, 1, 1_000_000_000_000).unwrap();
    assert!(q.not_enough_liquidity);
    let max_in = q.amount;
    assert!(max_in > 0 && q.expected_output <= 2_000_000);
    let (_, ub) = fx.venue.bounds(0, 1).unwrap();
    assert!(ub <= max_in && max_in - ub <= 100);
    assert_eq!(
        fx.swap(0, 1, max_in + 1),
        Err(ErrorCode::AmountOutExceedsBalance.code())
    );
    fx.refresh();
    let q = fx.check_swap(0, 1, max_in);
    assert_eq!(
        q.expected_output, 2_000_000,
        "the boundary drains the LP balance exactly"
    );
    assert_eq!(fx.pool().tokens[1].dynamics.actual_balance, 0);
    assert_eq!(fx.pool().tokens[1].dynamics.protocol_fees_owed, 50_000_000);
}

/// Deactivated input token, disabled pool, disabled swaps: the quote refuses
/// with the matching reason and the program reverts with the matching code.
#[test]
fn kill_switches() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(6, 4_000, 1_000_000_000_000, 1_000_000_000_000).inactive(),
            TokenSpec::new(6, 4_000, 1_000_000_000_000, 1_000_000_000_000),
            TokenSpec::new(6, 2_000, 500_000_000_000, 0), // live vb, no LP balance
        ],
    );
    // The declared directions are structural: every ordered pair, whether or
    // not it can trade right now (0 is inactive as INPUT, 2 has no actual
    // balance as OUTPUT — both are admin/LP-reversible states, answered by
    // `quote`, not by the declaration).
    assert_eq!(
        fx.venue.directions_num(),
        vec![(0, 1), (0, 2), (1, 0), (1, 2), (2, 0), (2, 1)]
    );
    assert!(matches!(
        fx.quote(0, 1, 1_000).unwrap_err(),
        TradingVenueError::AmmMethodError(_)
    ));
    assert!(
        fx.quote(0, 1, 0).is_err(),
        "no spot price for a paused input"
    );
    assert!(fx.venue.bounds(0, 1).is_err(), "no quotable range");
    assert_eq!(fx.swap(0, 1, 1_000), Err(ErrorCode::TokenInactive.code()));
    fx.refresh();
    // Buying the inactive token is fine.
    fx.check_swap(1, 0, 1_000_000);
    // Selling into the sidelined output token: only a dust input whose curve
    // output rounds to zero is accepted, so the partial fill has zero output.
    let q = fx.quote(1, 2, 1_000).unwrap();
    assert!(
        q.not_enough_liquidity && q.expected_output == 0 && q.amount < 1_000,
        "{q:?}"
    );
    assert_eq!(
        fx.swap(1, 2, 1_000),
        Err(ErrorCode::AmountOutExceedsBalance.code())
    );
    assert!(
        fx.venue.bounds(1, 2).is_err(),
        "no size yields output: no quotable range"
    );

    for (name, patch, code) in [
        (
            "pool disabled",
            (|p: &mut CofferPool| p.pool_enabled = false) as fn(&mut CofferPool),
            ErrorCode::PoolDisabled,
        ),
        (
            "swaps disabled",
            |p: &mut CofferPool| p.swaps_enabled = false,
            ErrorCode::SwapsDisabled,
        ),
    ] {
        let mut fx = Fixture::with_pool(
            3_000,
            &[
                TokenSpec::new(6, 5_000, 1_000_000_000_000, 1_000_000_000_000),
                TokenSpec::new(6, 5_000, 1_000_000_000_000, 1_000_000_000_000),
            ],
            patch,
        );
        assert!(
            matches!(
                fx.quote(0, 1, 1_000).unwrap_err(),
                TradingVenueError::InactivePoolError(..)
            ),
            "{name}"
        );
        assert!(matches!(
            fx.quote(0, 1, 0).unwrap_err(),
            TradingVenueError::InactivePoolError(..)
        ));
        assert_eq!(fx.swap(0, 1, 1_000), Err(code.code()), "{name}");
    }
}

/// A 10-token pool (the maximum) with mixed decimals, a Token-2022 mint, two
/// capped tokens and one surge curve: several directions match on-chain.
#[test]
fn ten_token_pool_many_directions() {
    let mut specs = vec![
        TokenSpec::new(9, 2_500, 30_000_000_000_000, 6_000_000_000_000),
        TokenSpec::new(6, 1_500, 5_000_000_000_000, 1_000_000_000_000).t22(),
        TokenSpec::new(6, 1_000, 5_000_000_000_000, 1_000_000_000_000)
            .cap(2_000)
            .surge(5_000, 100, 700, 3_000, 75),
        TokenSpec::new(8, 1_000, 200_000_000_000, 40_000_000_000).cap(1_500),
        TokenSpec::new(5, 800, 1_000_000_000_000, 300_000_000_000),
        TokenSpec::new(9, 800, 900_000_000_000_000, 100_000_000_000_000),
        TokenSpec::new(0, 700, 1_000_000, 300_000),
        TokenSpec::new(6, 700, 7_000_000_000_000, 2_000_000_000_000).t22(),
        TokenSpec::new(9, 500, 3_000_000_000_000, 1_000_000_000_000),
        TokenSpec::new(2, 500, 10_000_000_000, 4_000_000_000),
    ];
    assert_eq!(specs.iter().map(|s| s.weight).sum::<u64>(), 10_000);
    specs[0].protocol_fees_owed = 12_345;
    let mut fx = Fixture::new(4_000, &specs);
    assert_eq!(fx.venue.get_token_info().len(), 10);
    assert!(fx.venue.get_token_info()[1].is_token_2022);
    assert_eq!(fx.venue.directions_num().len(), 90);

    let directions = [
        (0usize, 1usize),
        (1, 0),
        (2, 5),
        (5, 2),
        (3, 6),
        (6, 9),
        (9, 3),
        (7, 4),
        (4, 8),
        (8, 7),
        (2, 3),
        (3, 2),
    ];
    for (i, j) in directions {
        fx.sweep(i, j, 5);
    }
    // The capped slots' windows have advanced exactly as predicted (checked
    // by check_swap through the post-state comparison); confirm they are live.
    assert!(fx.pool().tokens[2].dynamics.current_selloff > 0);
    assert!(fx.pool().tokens[3].dynamics.current_selloff > 0);
}

/// On the advertised domain the contract charges no surge, so prices and
/// outputs satisfy the shared MVT tolerance without a surge residual budget.
/// Zero LP fee isolates this policy from input-fee rounding.
#[test]
fn zero_surge_domain_satisfies_pricing_invariants() {
    for (name, w_in, w_out, threshold) in [
        ("50/50", 5_000, 5_000, 8_000),
        ("80/20", 8_000, 2_000, 6_000),
        ("20/80 threshold zero", 2_000, 8_000, 0),
        ("95/5 steep kink", 9_500, 500, 0),
    ] {
        let fx = Fixture::new(
            0,
            &[
                TokenSpec::new(9, w_in, 2_000_000_000_000, 2_000_000_000_000)
                    .cap(5_000)
                    .surge(threshold, 0, 1_000, 10_000, 1),
                TokenSpec::new(9, w_out, 8_000_000_000_000, 8_000_000_000_000),
            ],
        );
        let (lb, ub) = fx.venue.bounds(0, 1).unwrap();
        let points = grid(lb, ub, 128);
        for &x in &points {
            let q = fx.quote(0, 1, x).unwrap();
            assert!(!q.not_enough_liquidity);
            assert!(q.price.is_finite() && q.price > 0.0);
            let raw = quote_exact_in(&fx.pool(), x, 0, 1, 9, 9, fx.now()).unwrap();
            assert_eq!(raw.surge_fee_amount, 0, "{name}: input {x}");
            assert_eq!(raw.amount_out_user, q.expected_output);
        }
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (qa, qb) = (fx.quote(0, 1, a).unwrap(), fx.quote(0, 1, b).unwrap());
            assert!(
                qb.expected_output >= qa.expected_output,
                "{name}: output fell on [{a}, {b}]"
            );
            assert!(
                qb.price <= qa.price * (1.0 + 1e-5),
                "{name}: price increased"
            );
            let chord = (qb.expected_output - qa.expected_output) as f64 / (b - a) as f64;
            let atol = 2.0 / (b - a) as f64;
            assert!(
                chord <= qa.price * (1.0 + 1e-5) + atol,
                "{name}: chord above left price [{a}, {b}]"
            );
            assert!(
                chord >= qb.price * (1.0 - 1e-5) - atol,
                "{name}: chord below right price [{a}, {b}]"
            );
        }
    }
}

/// Quote latency on the fixtures (surge on and off), reported.
#[test]
fn quote_latency_report() {
    for (name, cap) in [("plain", 0u16), ("cap + surge", 5_000)] {
        let mut t0 = TokenSpec::new(6, 5_000, 2_000_000_000_000, 2_000_000_000_000);
        if cap > 0 {
            t0 = t0.cap(cap).surge(1, 100, 1_000, 3_000, 50);
        }
        let fx = Fixture::new(
            3_000,
            &[
                t0,
                TokenSpec::new(9, 5_000, 8_000_000_000_000, 8_000_000_000_000),
            ],
        );
        let (lb, ub) = fx.venue.bounds(0, 1).unwrap();
        let amounts = grid(lb, ub, 20_000);
        let req = |x| fx.request(0, 1, x);
        let start = Instant::now();
        let mut acc = 0u64;
        for &x in &amounts {
            acc = acc.wrapping_add(fx.venue.quote(req(x)).unwrap().expected_output);
        }
        let avg = start.elapsed().as_secs_f64() / amounts.len() as f64;
        eprintln!("{name}: average quote {:.0} ns ({acc})", avg * 1e9);
    }
}

/// Once an external swap enters the surge zone, the same venue must stop
/// offering positive fills. A zero-input quote is only a finite sentinel;
/// clock rotation later restores the direction without reconstructing it.
#[test]
fn surge_exhaustion_and_clock_recovery_on_same_venue() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(9, 5_000, 2_000_000_000_000, 2_000_000_000_000)
                .cap(5_000)
                .surge(5_000, 1_000, 2_000, 5_000, 75),
            TokenSpec::new(9, 5_000, 8_000_000_000_000, 8_000_000_000_000),
        ],
    );
    fx.check_raw_swap(0, 1, 700_000_000_000);
    assert_eq!(fx.venue.fill_limit(0, 1), Some(0));
    let q = fx.quote(0, 1, 1).unwrap();
    assert!(q.not_enough_liquidity);
    assert_eq!((q.amount, q.expected_output), (0, 0));
    assert!(fx.venue.bounds(0, 1).is_err());
    let spot = fx.quote(0, 1, 0).unwrap();
    assert_eq!((spot.amount, spot.expected_output), (0, 0));
    assert!(spot.price.is_finite());
    assert!(fx.venue.directions_num().contains(&(0, 1)));
    fx.set_clock(NOW + 2 * PERIOD as i64);
    fx.refresh();
    assert!(fx.venue.fill_limit(0, 1).unwrap() > 0);
    fx.check_swap(0, 1, 1_000_000);
}

/// Reproduce the measured downward output steps against the ELF, then prove
/// both offending inputs are outside the adapter's advertised domain.
#[test]
fn contract_output_declines_are_excluded_from_titan_quotes() {
    for (label, cap_pct, mid, high, kink, a, expected_a, b, expected_b) in [
        (
            "95/5 broad decline",
            5_000,
            0,
            2_500,
            0,
            400_000_000_000,
            963_537_089_473,
            490_000_000_000,
            960_973_264_083,
        ),
        (
            "95/5 steep kink",
            10_000,
            10_000,
            10_000,
            1,
            9_599_999_999,
            90_138_673_999,
            9_600_999_999,
            88_709_857_064,
        ),
    ] {
        assert!(b > a && expected_b < expected_a);
        for (amount, expected) in [(a, expected_a), (b, expected_b)] {
            let mut fx = Fixture::new(
                0,
                &[
                    TokenSpec::new(9, 9_500, 1_000_000_000_000, 1_000_000_000_000)
                        .cap(cap_pct)
                        .surge(0, 0, mid, high, kink),
                    TokenSpec::new(9, 500, 1_000_000_000_000, 1_000_000_000_000),
                ],
            );
            let limit = fx.venue.fill_limit(0, 1).unwrap();
            assert!(limit < a, "{label}: incompatible inputs remain advertised");
            let q = fx.quote(0, 1, amount).unwrap();
            assert!(q.not_enough_liquidity);
            assert_eq!(q.amount, limit);
            let (_, ub) = fx.venue.bounds(0, 1).unwrap();
            assert!(ub <= limit);
            assert_eq!(
                fx.check_raw_swap(0, 1, amount).amount_out_user,
                expected,
                "{label}"
            );
        }
    }
}

/// Cached curve and policy parameters must be rebuilt from every pool/clock
/// update. Snapshot writes here isolate adapter refresh from management
/// authorization; the local stand covers the actual RM/admin instructions.
#[test]
fn policy_refresh_tracks_virtual_balances_weights_admin_and_token_activity() {
    let mut fx = Fixture::new(
        3_000,
        &[
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000)
                .cap(5_000)
                .surge(5_000, 100, 1_000, 3_000, 75),
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000),
        ],
    );
    assert_eq!(fx.venue.fill_limit(0, 1), Some(250_049_999_999));
    fx.check_swap(0, 1, 100_000_000_000);
    let limit_before_rm = fx.venue.fill_limit(0, 1);
    let price_before_rm = fx.quote(0, 1, 1_000_000).unwrap().price;
    fx.update_pool(|pool| {
        pool.range_manager_enabled = true;
        pool.tokens[0].dynamics.virtual_balance = 2_000_000_000_000;
        pool.tokens[1].dynamics.virtual_balance = 2_000_000_000_000;
        pool.tokens[0].config.normalized_weight = 6_000;
        pool.tokens[1].config.normalized_weight = 4_000;
    });
    // The running window retains its pre-RM snapshot despite the new curve.
    assert_eq!(fx.venue.fill_limit(0, 1), limit_before_rm);
    assert_ne!(fx.quote(0, 1, 1_000_000).unwrap().price, price_before_rm);
    fx.check_swap(0, 1, 1_000_000);
    fx.update_pool(|pool| pool.tokens[0].config.variable_fee_threshold_pct = 2_000);
    assert_eq!(fx.venue.fill_limit(0, 1), Some(48_999_999));
    fx.update_pool(|pool| pool.tokens[0].config.is_active = false);
    assert!(fx.quote(0, 1, 1_000).is_err());
    fx.check_swap(1, 0, 1_000_000); // input kill switch still permits buying it
    fx.update_pool(|pool| pool.tokens[0].config.is_active = true);
    fx.check_swap(0, 1, 1_000_000);
    fx.update_pool(|pool| pool.tokens[0].config.variable_fee_slope_high_pct = 0);
    assert_eq!(fx.venue.fill_limit(0, 1), None);
    assert!(
        !fx.quote(0, 1, 100_000_000_000)
            .unwrap()
            .not_enough_liquidity
    );
    fx.update_pool(|pool| {
        pool.tokens[0].config.variable_fee_slope_high_pct = 3_000;
        pool.tokens[0].config.max_selloff_pct = 0;
    });
    assert_eq!(fx.venue.fill_limit(0, 1), None);
    fx.check_swap(0, 1, 100_000_000);
    fx.update_pool(|pool| pool.tokens[0].config.max_selloff_pct = 5_000);
    assert!(fx.venue.fill_limit(0, 1).is_some());
    // A fresh window adopts the moved VB; no stale cap or weight cache.
    let vb = fx.pool().tokens[0].dynamics.virtual_balance;
    fx.set_clock(NOW + 2 * PERIOD as i64);
    fx.refresh();
    let cap = vb / 2;
    let expected = ((cap as u128 * 2_001 - 1) / 10_000) as u64;
    assert_eq!(fx.venue.fill_limit(0, 1), Some(expected));
    fx.check_swap(0, 1, expected);
}

/// A one-period rotation re-bases the zero-surge boundary to the moved VB
/// and rescales the previous sell-off before applying its time decay.
#[test]
fn zero_surge_limit_uses_rescaled_carryover_after_range_update() {
    let mut fx = Fixture::new(
        0,
        &[
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000)
                .cap(5_000)
                .surge(5_000, 100, 1_000, 3_000, 75),
            TokenSpec::new(9, 5_000, 1_000_000_000_000, 1_000_000_000_000),
        ],
    );
    fx.check_swap(0, 1, 100_000_000_000);
    fx.update_pool(|pool| pool.tokens[0].dynamics.virtual_balance = 2_000_000_000_000);
    assert_eq!(fx.venue.fill_limit(0, 1), Some(150_049_999_999));
    let opened = NOW - 1_000;
    fx.set_clock(opened + PERIOD as i64);
    fx.refresh();
    // cap=1e12, previous=2e11, zero-surge endpoint=500,099,999,999.
    assert_eq!(fx.venue.fill_limit(0, 1), Some(300_099_999_999));
    fx.set_clock(opened + PERIOD as i64 + PERIOD as i64 / 2);
    fx.refresh();
    assert_eq!(fx.venue.fill_limit(0, 1), Some(400_099_999_999));
    fx.check_swap(0, 1, 400_099_999_999);
    let dynamics = fx.pool().tokens[0].dynamics;
    assert_eq!(dynamics.selloff_vb_snapshot, 2_000_000_000_000);
    assert_eq!(dynamics.previous_selloff, 200_000_000_000);
    assert_eq!(fx.venue.fill_limit(0, 1), Some(0));
}

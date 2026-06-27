//! Shared, venue-generic test suite.
//!
//! `tests/quay.rs` runs *these* functions against `QuayVenue`. Keeping the
//! assertions here (generic over the venue type) makes this module the single
//! source of truth for what a Titan venue must satisfy.
//!
//! Every function gates on prerequisites and SKIPs (returns) when they're
//! missing, so `cargo test` is clean on a fresh clone:
//! - all need a mainnet `SOLANA_RPC_URL`;
//! - the simulation checks additionally need the venue's program binaries dumped
//!   to `programs/<id>.so` (run `make dump-programs`).

#![allow(dead_code)] // each test binary exercises a subset of these helpers.

use std::env;
use std::path::Path;
use std::time::Instant;

use litesvm::LiteSVM;
use solana_account::{Account, ReadableAccount, WritableAccount};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_compute_budget::compute_budget::ComputeBudget;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sysvar::clock::{self, Clock};
use solana_transaction::Transaction;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token::state::{Account as TokenAccount, AccountState};

use assert_no_alloc::assert_no_alloc;

use titan_integration_template::account_caching::AccountsCache;
use titan_integration_template::account_caching::rpc_cache::RpcClientCache;
use titan_integration_template::trading_venue::{
    FromAccount, QuoteRequest, SwapType, TradingVenue,
};

/// Bound shared by every suite function: a venue that can be built from an
/// account and quoted, usable across `.await` points.
pub trait SuiteVenue: TradingVenue + FromAccount + Send + Sync {}
impl<T: TradingVenue + FromAccount + Send + Sync> SuiteVenue for T {}

/// Per-venue configuration the test entry points supply.
pub struct SuiteConfig {
    /// Pool/market account address the venue is constructed from.
    pub pool: Pubkey,
    /// Program binaries to load into LiteSVM for the swap simulation (the
    /// venue's own program plus any runtime dependencies). Each must be dumped
    /// to `programs/<id>.so` — see `make dump-programs`.
    pub programs: Vec<Pubkey>,
}

// ---------------------------------------------------------------------------
// Prerequisite gates and small helpers
// ---------------------------------------------------------------------------

pub fn init_test_logger() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn current_test() -> String {
    std::thread::current()
        .name()
        .unwrap_or("a venue test")
        .to_string()
}

/// RPC URL for the suite, or `None` (with a SKIP message) when `SOLANA_RPC_URL`
/// is unset — so the tests are no-ops on a fresh clone instead of panicking.
fn rpc_url_or_skip() -> Option<String> {
    match env::var("SOLANA_RPC_URL") {
        Ok(url) => Some(url),
        Err(_) => {
            eprintln!(
                "SKIP {}: set SOLANA_RPC_URL to run this venue test",
                current_test()
            );
            None
        }
    }
}

/// Whether every program binary the simulation needs is present in `programs/`.
fn programs_ready(programs: &[Pubkey]) -> bool {
    for id in programs {
        let path = format!("programs/{id}.so");
        if !Path::new(&path).exists() {
            eprintln!(
                "SKIP {}: missing {path} — run `make dump-programs` to fetch program binaries",
                current_test()
            );
            return false;
        }
    }
    true
}

/// Default seed for the sampling tests; override with `TEST_SEED=<u64>`.
const DEFAULT_TEST_SEED: u64 = 0x7174_616e_5345_4544; // "titanSED"

/// Deterministic RNG for sampling-based tests. Seeded from `TEST_SEED` (default
/// `DEFAULT_TEST_SEED`) and printed so any failure is reproducible.
fn test_rng() -> rand::rngs::StdRng {
    use rand::SeedableRng;
    let seed = env::var("TEST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TEST_SEED);
    eprintln!("{}: TEST_SEED={seed}", current_test());
    rand::rngs::StdRng::seed_from_u64(seed)
}

/// Log-uniform sample in `[lo, hi]`, drawn from the supplied seeded RNG.
fn sample_log_uniform_u64(rng: &mut rand::rngs::StdRng, lo: u64, hi: u64) -> u64 {
    use rand::Rng;
    assert!(lo >= 1, "log-uniform sampling requires lo >= 1");
    assert!(lo <= hi);
    let log_lo = (lo as f64).ln();
    let log_hi = (hi as f64).ln();
    let r: f64 = rng.random();
    ((log_lo + r * (log_hi - log_lo)).exp() as u64).clamp(lo, hi)
}

/// Geometrically-spaced probe points across `[lb, ub]`, for the mean-value
/// theorem chord checks.
fn geometric_grid(lb: u64, ub: u64, n: usize) -> Vec<u64> {
    assert!(lb >= 1 && ub > lb && n >= 2);
    let ln_lo = (lb as f64).ln();
    let ln_hi = (ub as f64).ln();
    let mut points: Vec<u64> = (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64;
            ((ln_lo + t * (ln_hi - ln_lo)).exp() as u64).clamp(lb, ub)
        })
        .collect();
    points.sort();
    points.dedup();
    points
}

fn exact_in(input_mint: Pubkey, output_mint: Pubkey, amount: u64) -> QuoteRequest {
    QuoteRequest {
        input_mint,
        output_mint,
        amount,
        swap_type: SwapType::ExactIn,
    }
}

/// Build a LiteSVM loaded with the given programs and a funded payer.
fn setup_litesvm(programs: &[Pubkey]) -> (LiteSVM, Keypair) {
    let mut litesvm = LiteSVM::new()
        .with_compute_budget(ComputeBudget {
            compute_unit_limit: 1_400_000,
            ..Default::default()
        })
        .with_blockhash_check(false)
        .with_sigverify(false)
        .with_transaction_history(0);

    for id in programs {
        litesvm
            .add_program_from_file(*id, format!("programs/{id}.so"))
            .unwrap_or_else(|_| panic!("failed to load programs/{id}.so"));
    }

    let keypair = Keypair::new();
    let account = Account {
        lamports: 10_000 * LAMPORTS_PER_SOL,
        data: vec![],
        owner: solana_sdk::system_program::id(),
        executable: false,
        rent_epoch: 0,
    };
    litesvm.set_account(keypair.pubkey(), account).unwrap();
    (litesvm, keypair)
}

/// Sync LiteSVM's clock sysvar to the live network so time-dependent venues
/// quote against the same clock the simulation runs under.
async fn sync_clock(cache: &RpcClientCache, litesvm: &mut LiteSVM) {
    let clock_account = cache.get_account(&clock::ID).await.unwrap();
    let clock: Clock = clock_account
        .as_ref()
        .expect("clock sysvar account")
        .deserialize_data()
        .unwrap();
    litesvm.set_sysvar::<Clock>(&clock);
}

/// Fetch the pool, build the venue, and bring it to a fully-updated state.
/// Returns the venue plus the RPC cache it was loaded through (reused for sims).
async fn build_venue<V: SuiteVenue>(rpc_url: String, pool: Pubkey) -> (V, RpcClientCache) {
    let rpc = RpcClient::new(rpc_url);
    let account = rpc
        .get_account(&pool)
        .await
        .expect("failed to fetch pool account");
    let mut venue = V::from_account(&pool, &account).expect("failed to build venue from account");
    let cache = RpcClientCache::new(rpc);
    venue
        .update_state(&cache)
        .await
        .expect("venue state update failed");
    (venue, cache)
}

/// Execute a swap through the venue's generated instruction inside LiteSVM and
/// return the realized output amount — the on-chain ground truth to compare a
/// quote against.
async fn sim_quote_request(
    venue: &dyn TradingVenue,
    cache: &dyn AccountsCache,
    request: QuoteRequest,
    litesvm: &mut LiteSVM,
    keypair: &Keypair,
) -> u64 {
    let tokens = venue.get_token_info();
    let idx_0 = tokens
        .iter()
        .position(|t| t.pubkey == request.input_mint)
        .expect("input mint not in venue");
    let idx_1 = tokens
        .iter()
        .position(|t| t.pubkey == request.output_mint)
        .expect("output mint not in venue");

    let (token_a, token_a_program) = (tokens[idx_0].pubkey, tokens[idx_0].get_token_program());
    let (token_b, token_b_program) = (tokens[idx_1].pubkey, tokens[idx_1].get_token_program());

    let token_account_a =
        get_associated_token_address_with_program_id(&keypair.pubkey(), &token_a, &token_a_program);
    let token_account_b =
        get_associated_token_address_with_program_id(&keypair.pubkey(), &token_b, &token_b_program);

    // Source account funded "infinitely"; destination starts empty.
    let mut account_a = Account::new(LAMPORTS_PER_SOL, TokenAccount::LEN, &token_a_program);
    let a = TokenAccount {
        mint: token_a,
        owner: keypair.pubkey(),
        state: AccountState::Initialized,
        amount: u64::MAX,
        ..Default::default()
    };
    a.pack_into_slice(account_a.data_as_mut_slice());

    let mut account_b = Account::new(LAMPORTS_PER_SOL, TokenAccount::LEN, &token_b_program);
    let b = TokenAccount {
        mint: token_b,
        owner: keypair.pubkey(),
        state: AccountState::Initialized,
        amount: 0,
        ..Default::default()
    };
    b.pack_into_slice(account_b.data_as_mut_slice());

    litesvm.set_account(token_account_a, account_a).unwrap();
    litesvm.set_account(token_account_b, account_b).unwrap();

    let ix = venue
        .generate_swap_instruction(request, keypair.pubkey())
        .unwrap();

    // Load every non-executable instruction account from the cache into the SVM.
    let pks: Vec<Pubkey> = ix.accounts.iter().map(|a| a.pubkey).collect();
    let loaded = cache.get_accounts(&pks).await.unwrap();
    for (account, key) in loaded.into_iter().zip(pks) {
        if let Some(acc) = account
            && !acc.executable
        {
            litesvm.set_account(key, acc).unwrap();
        }
    }

    let blockhash = litesvm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[ix], Some(&keypair.pubkey()), &[keypair], blockhash);
    let result = litesvm.simulate_transaction(tx).unwrap();

    let post = result
        .post_accounts
        .into_iter()
        .find(|(pk, _)| pk == &token_account_b)
        .map(|(_, acc)| acc)
        .expect("output token account missing from simulation");
    TokenAccount::unpack_from_slice(post.data())
        .expect("failed to unpack output token account")
        .amount
}

// ---------------------------------------------------------------------------
// The suite. Each function is one venue test; the entry points wrap these in
// `#[tokio::test]` against their venue type.
// ---------------------------------------------------------------------------

/// Construction & boundaries: the venue builds, loads state, exposes valid token
/// info, computes boundaries with a positive spot price, and quotes (with a
/// positive price) at both edges — all without allocating in the quote path.
pub async fn construction<V: SuiteVenue>(config: &SuiteConfig) {
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    let (venue, _cache) = build_venue::<V>(rpc_url, config.pool).await;

    let token_info = venue.get_token_info();
    log::info!("Loaded token info: {:#?}", token_info);
    assert!(
        token_info.len() >= 2,
        "venue must expose at least two tokens"
    );

    for (in_idx, out_idx) in venue.directions_num() {
        let (lower, upper) =
            assert_no_alloc(|| venue.bounds(in_idx, out_idx)).expect("boundary search failed");
        assert!(lower < upper, "lower bound must be < upper bound");

        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        for (edge, amount) in [("lower", lower), ("upper", upper)] {
            let q = assert_no_alloc(|| venue.quote(exact_in(input_mint, output_mint, amount)))
                .unwrap_or_else(|_| panic!("{edge}-bound quote failed"));
            assert!(
                !q.not_enough_liquidity,
                "{edge} bound: insufficient liquidity"
            );
            assert!(q.expected_output > 0, "{edge} bound: zero output");
            assert!(
                q.price > 0.0,
                "{edge} bound: non-positive price {}",
                q.price
            );
        }
    }
}

/// Zero-input quote: Titan sometimes requests a quote at `amount == 0`. The
/// venue must not error — it must return zero output together with a positive
/// spot price `f'(0)`. This is the boundary case of the pricing contract on
/// `QuoteResult::price`.
pub async fn zero_input_spot_price<V: SuiteVenue>(config: &SuiteConfig) {
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    let (venue, _cache) = build_venue::<V>(rpc_url, config.pool).await;
    assert!(venue.get_token_info().len() >= 2);

    for (in_idx, out_idx) in venue.directions_num() {
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        let quote = venue
            .quote(exact_in(input_mint, output_mint, 0))
            .expect("zero-input quote must not error");
        assert_eq!(
            quote.expected_output, 0,
            "zero input must produce zero output"
        );
        assert!(
            quote.price > 0.0,
            "zero input must still report a positive spot price, got {}",
            quote.price
        );
    }
}

/// Boundary simulation: the off-chain quote matches on-chain execution exactly
/// at both boundary edges, in every declared direction.
pub async fn bound_simulation<V: SuiteVenue>(config: &SuiteConfig) {
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    if !programs_ready(&config.programs) {
        return;
    }
    let (venue, cache) = build_venue::<V>(rpc_url, config.pool).await;
    let (mut litesvm, keypair) = setup_litesvm(&config.programs);
    sync_clock(&cache, &mut litesvm).await;

    assert!(venue.get_token_info().len() >= 2);

    for (in_idx, out_idx) in venue.directions_num() {
        let (lower, upper) = venue.bounds(in_idx, out_idx).unwrap();
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        for bound in [lower, upper] {
            let request = exact_in(input_mint, output_mint, bound);
            let sim =
                sim_quote_request(&venue, &cache, request.clone(), &mut litesvm, &keypair).await;
            let quote = venue.quote(request).unwrap();
            assert_eq!(
                quote.expected_output.abs_diff(sim),
                0,
                "quote {} != sim {} at bound {bound}",
                quote.expected_output,
                sim
            );
        }
    }
}

/// Random-sample simulation: across the whole valid range, the off-chain quote
/// matches on-chain execution for every declared direction.
pub async fn random_samples<V: SuiteVenue>(config: &SuiteConfig) {
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    if !programs_ready(&config.programs) {
        return;
    }
    let (venue, cache) = build_venue::<V>(rpc_url, config.pool).await;
    let (mut litesvm, keypair) = setup_litesvm(&config.programs);
    sync_clock(&cache, &mut litesvm).await;

    let mut rng = test_rng();
    for (in_idx, out_idx) in venue.directions_num() {
        let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        for _ in 0..50 {
            let amount = sample_log_uniform_u64(&mut rng, lb, ub);
            let request = exact_in(input_mint, output_mint, amount);
            let sim =
                sim_quote_request(&venue, &cache, request.clone(), &mut litesvm, &keypair).await;
            let quote = venue.quote(request).unwrap();
            assert_eq!(
                quote.expected_output.abs_diff(sim),
                0,
                "quote {} != sim {} at amount {amount}",
                quote.expected_output,
                sim
            );
        }
    }
}

/// Output monotonicity: a larger `ExactIn` amount never returns less output.
pub async fn monotone<V: SuiteVenue>(config: &SuiteConfig) {
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    let (venue, _cache) = build_venue::<V>(rpc_url, config.pool).await;

    let mut rng = test_rng();
    for (in_idx, out_idx) in venue.directions_num() {
        let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        let mut amounts: Vec<u64> = (0..50)
            .map(|_| sample_log_uniform_u64(&mut rng, lb, ub))
            .collect();
        amounts.sort();

        let mut prev = 0;
        for amount in amounts {
            let out = venue
                .quote(exact_in(input_mint, output_mint, amount))
                .expect("quote failed")
                .expected_output;
            assert!(
                prev <= out,
                "output not monotone: {prev} > {out} at {amount}"
            );
            prev = out;
        }
    }
}

/// Quoting speed: a single quote must average under 1 microsecond (1µs) so the
/// router can evaluate venues in real time.
pub async fn quoting_speed<V: SuiteVenue>(config: &SuiteConfig) {
    const ITERATIONS: usize = 10_000;
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    let (venue, _cache) = build_venue::<V>(rpc_url, config.pool).await;

    let mut rng = test_rng();
    for (in_idx, out_idx) in venue.directions_num() {
        let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        let amounts: Vec<u64> = (0..ITERATIONS)
            .map(|_| sample_log_uniform_u64(&mut rng, lb, ub))
            .collect();
        let start = Instant::now();
        for amount in amounts {
            let _ = venue
                .quote(exact_in(input_mint, output_mint, amount))
                .expect("quote failed");
        }
        let avg = start.elapsed().as_secs_f64() / ITERATIONS as f64;
        log::info!("average quote time: {avg}s");
        assert!(
            avg < 0.000001,
            "quoting too slow ({avg}s) for {input_mint} -> {output_mint}"
        );
    }
}

/// Price monotonicity (concavity): the reported marginal price is positive and
/// non-increasing as the input grows.
pub async fn price_monotone<V: SuiteVenue>(config: &SuiteConfig) {
    const REL_TOL: f64 = 1e-3; // slack so integer rounding can't look like a violation
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    let (venue, _cache) = build_venue::<V>(rpc_url, config.pool).await;
    assert!(venue.get_token_info().len() >= 2);

    let mut rng = test_rng();
    for (in_idx, out_idx) in venue.directions_num() {
        let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        let mut amounts: Vec<u64> = (0..200)
            .map(|_| sample_log_uniform_u64(&mut rng, lb, ub))
            .collect();
        amounts.sort();

        let mut prev_price = f64::INFINITY;
        for amount in amounts {
            let price = venue
                .quote(exact_in(input_mint, output_mint, amount))
                .expect("quote failed")
                .price;
            assert!(
                price > 0.0,
                "price must be positive, got {price} at {amount}"
            );
            assert!(
                price <= prev_price * (1.0 + REL_TOL),
                "price not monotone non-increasing: {prev_price} -> {price} at {amount}"
            );
            prev_price = price;
        }
    }
}

/// Mean value theorem: the realized chord of the output curve is bracketed by
/// the reported endpoint prices, certifying the price is the genuine derivative
/// of the quoted output.
pub async fn mean_value_theorem<V: SuiteVenue>(config: &SuiteConfig) {
    const REL_TOL: f64 = 1e-5; // 0.1 BPS
    const OUT_QUANTUM: f64 = 2.0; // two output atoms of floor-truncation slack
    init_test_logger();
    let Some(rpc_url) = rpc_url_or_skip() else {
        return;
    };
    let (venue, _cache) = build_venue::<V>(rpc_url, config.pool).await;
    assert!(venue.get_token_info().len() >= 2);

    for (in_idx, out_idx) in venue.directions_num() {
        let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
        let input_mint = venue.get_token(in_idx as usize).unwrap().pubkey;
        let output_mint = venue.get_token(out_idx as usize).unwrap().pubkey;

        let grid = geometric_grid(lb, ub, 64);
        for pair in grid.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b <= a {
                continue;
            }
            let qa = venue
                .quote(exact_in(input_mint, output_mint, a))
                .expect("quote at a");
            let qb = venue
                .quote(exact_in(input_mint, output_mint, b))
                .expect("quote at b");
            if qb.expected_output <= qa.expected_output {
                continue; // flat step carries no rate information
            }

            let chord = (qb.expected_output - qa.expected_output) as f64 / (b - a) as f64;
            let (price_a, price_b) = (qa.price, qb.price); // f'(a) >= f'(b)
            let atol = OUT_QUANTUM / (b - a) as f64;

            assert!(
                price_b <= price_a * (1.0 + REL_TOL),
                "price increased with size: f'({a})={price_a} < f'({b})={price_b}"
            );
            assert!(
                chord <= price_a * (1.0 + REL_TOL) + atol,
                "chord {chord} exceeds left price {price_a} (atol {atol}) on [{a}, {b}]"
            );
            assert!(
                chord >= price_b * (1.0 - REL_TOL) - atol,
                "chord {chord} below right price {price_b} (atol {atol}) on [{a}, {b}]"
            );
        }
    }
}

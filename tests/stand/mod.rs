//! Shared helpers for the local-validator stand tests
//! (`local_stand_matrix.rs`, `local_stand_range_manager.rs`): stand access,
//! the quote-vs-real-transaction parity check, and running one of Titan's
//! shared-suite functions on a pool of the stand.

#![allow(dead_code)] // each test binary uses a subset

use std::str::FromStr;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_instruction::Instruction;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signature::Signature;
use solana_sdk::signature::{Keypair, Signer, read_keypair_file};
use solana_sdk::transaction::{Transaction, TransactionError};
use solana_transaction_status_client_types::{
    UiTransactionEncoding, option_serializer::OptionSerializer,
};
use spl_token::state::Account as TokenAccount;

use titan_integration_template::account_caching::rpc_cache::RpcClientCache;
use titan_integration_template::coffer::errors::ErrorCode;
use titan_integration_template::coffer::state::CofferPool;
use titan_integration_template::coffer::swap::{
    SwapOutcome, apply_swap, quote_exact_in, selloff_headroom,
};
use titan_integration_template::coffer_venue::{COFFER_PROGRAM_ID, CofferVenue};
use titan_integration_template::local_stand::*;
use titan_integration_template::trading_venue::{
    FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
};

use crate::common::{self, SuiteConfig};

// ---------------------------------------------------------------------------
// Stand access
// ---------------------------------------------------------------------------

pub struct Stand {
    pub manifest: StandManifest,
    pub rpc: RpcClient,
    pub wallet: Keypair,
}

pub async fn stand_or_skip(test: &str) -> Option<Stand> {
    let Some(manifest) = StandManifest::load() else {
        eprintln!(
            "SKIP {test}: no {} — run scripts/local-stand/up.sh",
            StandManifest::path().display()
        );
        return None;
    };
    let Ok(url) = std::env::var("SOLANA_RPC_URL") else {
        eprintln!("SKIP {test}: set SOLANA_RPC_URL={}", manifest.rpc);
        return None;
    };
    let rpc = RpcClient::new_with_commitment(url, CommitmentConfig::confirmed());
    if rpc.get_health().await.is_err() {
        eprintln!("SKIP {test}: local validator not reachable");
        return None;
    }
    let wallet_path = std::env::var("LOCAL_STAND_WALLET")
        .unwrap_or_else(|_| format!("{}/.config/solana/id.json", std::env::var("HOME").unwrap()));
    let wallet = read_keypair_file(&wallet_path).expect("wallet keypair");
    assert_eq!(
        wallet.pubkey().to_string(),
        manifest.wallet,
        "wallet differs from the stand's pool admin"
    );
    Some(Stand {
        manifest,
        rpc,
        wallet,
    })
}

impl Stand {
    pub fn pool(&self, case: &str) -> &StandPool {
        self.manifest
            .pools
            .iter()
            .find(|p| p.case == case)
            .unwrap_or_else(|| panic!("pool {case} in manifest"))
    }

    pub fn mint(&self, name: &str) -> (Pubkey, Pubkey) {
        let m = self.manifest.mint(name);
        (
            Pubkey::from_str(&m.address).unwrap(),
            if m.token_2022 {
                spl_token_2022::ID
            } else {
                spl_token::ID
            },
        )
    }

    /// Fresh venue from live state (fresh cache: no stale accounts).
    pub async fn venue(&self, pool: Pubkey) -> CofferVenue {
        let account = self.rpc.get_account(&pool).await.expect("pool account");
        let mut venue = CofferVenue::from_account(&pool, &account).expect("from_account");
        let cache = RpcClientCache::new(RpcClient::new_with_commitment(
            self.manifest.rpc.clone(),
            CommitmentConfig::confirmed(),
        ));
        venue.update_state(&cache).await.expect("update_state");
        venue
    }

    pub async fn pool_state(&self, pool: Pubkey) -> CofferPool {
        CofferPool::from_account_data(&self.rpc.get_account(&pool).await.unwrap().data).unwrap()
    }

    pub async fn token_balance(&self, mint: &Pubkey, tp: &Pubkey) -> u64 {
        let key = ata(&self.wallet.pubkey(), mint, tp);
        match self.rpc.get_account(&key).await {
            Ok(acc) => TokenAccount::unpack_from_slice(&acc.data)
                .map(|a| a.amount)
                .unwrap_or(0),
            Err(_) => 0,
        }
    }

    /// Send and confirm; on failure return the program's custom error code.
    pub async fn send(&self, ixs: &[Instruction]) -> Result<Signature, u32> {
        let bh = self.rpc.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            ixs,
            Some(&self.wallet.pubkey()),
            &[&self.wallet],
            bh,
        );
        match self.rpc.send_and_confirm_transaction(&tx).await {
            Ok(sig) => Ok(sig),
            Err(e) => match e.get_transaction_error() {
                Some(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
                    Err(code)
                }
                other => panic!("transaction failed without a custom code: {other:?} / {e}"),
            },
        }
    }

    /// `(block_time, compute_units_consumed)` of a confirmed transaction.
    pub async fn tx_meta(&self, sig: &Signature) -> (i64, u64) {
        let tx = self
            .rpc
            .get_transaction_with_config(
                sig,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Json),
                    commitment: Some(CommitmentConfig::confirmed()),
                    max_supported_transaction_version: Some(0),
                },
            )
            .await
            .expect("confirmed transaction");
        let cu = tx
            .transaction
            .meta
            .and_then(|m| match m.compute_units_consumed {
                OptionSerializer::Some(v) => Some(v),
                _ => None,
            })
            .unwrap_or_default();
        (tx.block_time.unwrap_or(0), cu)
    }

    pub fn request(&self, venue: &CofferVenue, i: u8, j: u8, amount: u64) -> QuoteRequest {
        QuoteRequest {
            input_mint: venue.get_token(i as usize).unwrap().pubkey,
            output_mint: venue.get_token(j as usize).unwrap().pubkey,
            amount,
            swap_type: SwapType::ExactIn,
        }
    }

    /// Execute one swap for real (routed through the Titan router when
    /// `routed`, else the venue's own `swap` instruction) and compare the
    /// user's output delta and the post-swap pool account with the venue's
    /// prediction. When they differ, the quote is recomputed with the venue's
    /// clock set to the transaction's block time: the sell-off window is a
    /// function of the clock, so a rotation or carry-over decay between the
    /// quote and the landing block explains a difference exactly.
    pub async fn parity_swap(
        &self,
        venue: &CofferVenue,
        i: u8,
        j: u8,
        amount: u64,
        routed: bool,
    ) -> Parity {
        let request = self.request(venue, i, j, amount);
        let quote = venue.quote(request).expect("adapter quote");
        self.execute_parity_swap(
            venue,
            (i, j),
            amount,
            routed,
            ContractFill {
                amount: quote.amount,
                expected_output: quote.expected_output,
                not_enough_liquidity: quote.not_enough_liquidity,
            },
        )
        .await
    }

    /// Test the complete contract amount, including surge regimes that Titan
    /// deliberately does not advertise. This validates the raw port and the
    /// instruction/router execution, not Titan quote availability or price.
    pub async fn contract_parity_swap(
        &self,
        venue: &CofferVenue,
        i: u8,
        j: u8,
        amount: u64,
        routed: bool,
    ) -> Parity {
        let outcome = self
            .contract_quote(venue, i, j, amount)
            .expect("contract quote");
        self.execute_parity_swap(
            venue,
            (i, j),
            amount,
            routed,
            ContractFill {
                amount,
                expected_output: outcome.amount_out_user,
                not_enough_liquidity: false,
            },
        )
        .await
    }

    pub fn contract_quote(
        &self,
        venue: &CofferVenue,
        i: u8,
        j: u8,
        amount: u64,
    ) -> Result<SwapOutcome, ErrorCode> {
        quote_exact_in(
            venue.pool(),
            amount,
            i,
            j,
            venue.get_token(i as usize).unwrap().decimals as u8,
            venue.get_token(j as usize).unwrap().decimals as u8,
            venue.now(),
        )
    }

    async fn execute_parity_swap(
        &self,
        venue: &CofferVenue,
        direction: (u8, u8),
        amount: u64,
        routed: bool,
        quote: ContractFill,
    ) -> Parity {
        let (i, j) = direction;
        let request = self.request(venue, i, j, amount);
        if quote.amount == 0 {
            // Nothing fillable in this direction right now (window exhausted
            // by the previous sample): nothing to execute or compare.
            return Parity {
                amount_requested: amount,
                amount_executed: 0,
                partial: true,
                expected: 0,
                received: 0,
                error: None,
                pool_matches: true,
                surge_fee: 0,
                compute_units: 0,
                block_time: 0,
                clock_adjusted: None,
            };
        }
        let (dec_in, dec_out) = (
            venue.get_token(i as usize).unwrap().decimals as u8,
            venue.get_token(j as usize).unwrap().decimals as u8,
        );
        let predict = |now: i64| -> (u64, u64, CofferPool) {
            let outcome = quote_exact_in(venue.pool(), quote.amount, i, j, dec_in, dec_out, now)
                .expect("exact quote at the fillable amount");
            let mut predicted = *venue.pool();
            apply_swap(&mut predicted, i, j, &outcome).unwrap();
            (outcome.amount_out_user, outcome.surge_fee_amount, predicted)
        };
        let (expected, surge_fee, predicted) = predict(venue.now());
        assert_eq!(
            quote.expected_output, expected,
            "selected quote must match the exact port"
        );

        let out_tp = venue.get_token(j as usize).unwrap().get_token_program();
        let before = self.token_balance(&request.output_mint, &out_tp).await;
        let executed = QuoteRequest {
            amount: quote.amount,
            ..request.clone()
        };
        let ixs = if routed {
            // A routed coffer leg needs more than the router's 200k default:
            // the router spends ~45k CU creating the TitanPDA ATAs and the
            // 4-segment surge fee costs the swap four extra curve evaluations.
            vec![
                ComputeBudgetInstruction::set_compute_unit_limit(600_000),
                build_route_ix(venue, &executed, self.wallet.pubkey()).unwrap(),
            ]
        } else {
            vec![
                venue
                    .generate_swap_instruction(executed, self.wallet.pubkey())
                    .unwrap(),
            ]
        };
        let sent = self.send(&ixs).await;
        let after = self.token_balance(&request.output_mint, &out_tp).await;
        let chain = self.pool_state(venue.market_id()).await;
        let received = after - before;
        let (mut block_time, mut compute_units) = (0, 0);
        let mut clock_adjusted = None;
        if let Ok(sig) = &sent {
            let (bt, cu) = self.tx_meta(sig).await;
            block_time = bt;
            compute_units = cu;
            if (received != expected || chain != predicted) && bt != venue.now() {
                let (e2, _, p2) = predict(bt);
                clock_adjusted = Some(ClockAdjusted {
                    block_time_minus_now: bt - venue.now(),
                    expected: e2,
                    matches: received == e2 && chain == p2,
                });
            }
        }
        Parity {
            amount_requested: amount,
            amount_executed: quote.amount,
            partial: quote.not_enough_liquidity,
            expected,
            received,
            error: sent.err(),
            pool_matches: chain == predicted,
            surge_fee,
            compute_units,
            block_time,
            clock_adjusted,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClockAdjusted {
    pub block_time_minus_now: i64,
    pub expected: u64,
    pub matches: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Parity {
    pub amount_requested: u64,
    pub amount_executed: u64,
    pub partial: bool,
    pub expected: u64,
    pub received: u64,
    pub error: Option<u32>,
    pub pool_matches: bool,
    pub surge_fee: u64,
    pub compute_units: u64,
    pub block_time: i64,
    /// Set when the plain comparison failed and the venue's clock differed
    /// from the block time: the re-quote at the block time.
    pub clock_adjusted: Option<ClockAdjusted>,
}

impl Parity {
    /// Exact at the venue's clock.
    pub fn exact(&self) -> bool {
        self.error.is_none() && self.received == self.expected && self.pool_matches
    }
    /// Exact at the venue's clock, or exact once the venue clock is set to the
    /// block time (the window moved between quote and landing).
    pub fn ok(&self) -> bool {
        self.exact() || self.clock_adjusted.is_some_and(|c| c.matches)
    }
}

pub fn geometric(lb: u64, ub: u64, n: usize) -> Vec<u64> {
    let mut v: Vec<u64> = (0..n)
        .map(|k| {
            let t = k as f64 / (n - 1) as f64;
            (((lb as f64).ln() + t * ((ub as f64).ln() - (lb as f64).ln())).exp() as u64)
                .clamp(lb, ub)
        })
        .collect();
    v.dedup();
    v
}

pub fn headroom_of(venue: &CofferVenue, i: u8, j: u8) -> QuoteResult {
    venue
        .quote(QuoteRequest {
            input_mint: venue.get_token(i as usize).unwrap().pubkey,
            output_mint: venue.get_token(j as usize).unwrap().pubkey,
            amount: u64::MAX / 8,
            swap_type: SwapType::ExactIn,
        })
        .expect("probe quote")
}

/// Amount/output without a marginal-price claim. Used only by explicit
/// raw-contract tests; never passed to Titan as a venue quote.
pub struct ContractFill {
    pub amount: u64,
    pub expected_output: u64,
    pub not_enough_liquidity: bool,
}

/// Largest input the exact contract port accepts, independent of the
/// adapter's conservative surge-free routing domain.
pub fn contract_headroom_of(venue: &CofferVenue, i: u8, j: u8) -> ContractFill {
    let quote = |amount| {
        quote_exact_in(
            venue.pool(),
            amount,
            i,
            j,
            venue.get_token(i as usize).unwrap().decimals as u8,
            venue.get_token(j as usize).unwrap().decimals as u8,
            venue.now(),
        )
    };
    let mut hi = selloff_headroom(venue.pool(), i, venue.now())
        .expect("window configuration")
        .unwrap_or(u64::MAX / 8);
    let amount = if hi == 0 || quote(hi).is_ok() {
        hi
    } else {
        let mut lo = 0;
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if quote(mid).is_ok() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    ContractFill {
        amount,
        expected_output: if amount == 0 {
            0
        } else {
            quote(amount).unwrap().amount_out_user
        },
        not_enough_liquidity: amount < u64::MAX / 8,
    }
}

/// Check the adapter policy separately from full-contract execution. Bounds
/// and partial fills must agree with the cached limit after each mutation.
pub fn assert_adapter_domain(venue: &CofferVenue, i: u8, j: u8) {
    let Some(limit) = venue.fill_limit(i, j) else {
        return;
    };
    let request = |amount| QuoteRequest {
        input_mint: venue.get_token(i as usize).unwrap().pubkey,
        output_mint: venue.get_token(j as usize).unwrap().pubkey,
        amount,
        swap_type: SwapType::ExactIn,
    };
    if limit == 0 {
        let q = venue.quote(request(1)).unwrap();
        assert!(q.not_enough_liquidity && q.amount == 0 && q.expected_output == 0);
        assert!(venue.bounds(i, j).is_err());
        return;
    }
    let q = venue.quote(request(limit)).unwrap();
    assert!(!q.not_enough_liquidity && q.amount == limit);
    assert!(q.price > 0.0 && q.price.is_finite());
    let exact = quote_exact_in(
        venue.pool(),
        limit,
        i,
        j,
        venue.get_token(i as usize).unwrap().decimals as u8,
        venue.get_token(j as usize).unwrap().decimals as u8,
        venue.now(),
    )
    .unwrap();
    assert_eq!(exact.surge_fee_amount, 0);
    assert_eq!(q.expected_output, exact.amount_out_user);
    let beyond = venue.quote(request(limit + 1)).unwrap();
    assert!(beyond.not_enough_liquidity && beyond.amount == limit);
    assert_eq!(beyond.expected_output, q.expected_output);
}

pub const SUITE_TESTS: [&str; 8] = [
    "construction",
    "zero_input_spot_price",
    "bound_simulation",
    "random_samples",
    "monotone",
    "quoting_speed",
    "price_monotone",
    "mean_value_theorem",
];

/// Run one shared-suite function on its own OS thread with a private runtime
/// (the suite's futures hold a LiteSVM and are not `Send`), capturing a panic
/// as the failure message. With `quotable_only` the suite sees the venue
/// through `QuotableDirections`, i.e. only the directions that currently have
/// a quotable range (a pool with a deactivated token declares the paused
/// direction too, and the suite would otherwise fail on it by design).
pub async fn run_suite_test(name: &str, pool: Pubkey, quotable_only: bool) -> Result<(), String> {
    let name_owned = name.to_string();
    let handle = std::thread::spawn(move || {
        let config = SuiteConfig {
            pool,
            programs: vec![COFFER_PROGRAM_ID],
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            if quotable_only {
                run_named::<QuotableDirections<CofferVenue>>(&name_owned, &config).await
            } else {
                run_named::<CofferVenue>(&name_owned, &config).await
            }
        })
    });
    tokio::task::spawn_blocking(move || match handle.join() {
        Ok(()) => Ok(()),
        Err(payload) => Err(payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "panic".into())),
    })
    .await
    .unwrap()
}

async fn run_named<V: common::SuiteVenue>(name: &str, config: &SuiteConfig) {
    match name {
        "construction" => common::construction::<V>(config).await,
        "zero_input_spot_price" => common::zero_input_spot_price::<V>(config).await,
        "bound_simulation" => common::bound_simulation::<V>(config).await,
        "random_samples" => common::random_samples::<V>(config).await,
        "monotone" => common::monotone::<V>(config).await,
        "quoting_speed" => common::quoting_speed::<V>(config).await,
        "price_monotone" => common::price_monotone::<V>(config).await,
        "mean_value_theorem" => common::mean_value_theorem::<V>(config).await,
        other => panic!("unknown suite test {other}"),
    }
}

/// Run the whole shared suite on one pool; returns `(test, result)` rows.
pub async fn run_full_suite(
    pool: Pubkey,
    quotable_only: bool,
) -> Vec<(&'static str, Result<(), String>)> {
    let mut rows = Vec::new();
    for t in SUITE_TESTS {
        rows.push((t, run_suite_test(t, pool, quotable_only).await));
    }
    rows
}

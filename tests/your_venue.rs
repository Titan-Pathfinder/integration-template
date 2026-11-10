//! The Coffer `Coffer` venue's suite — the same shared assertions the
//! Raydium example passes, run against `CofferVenue` on several live
//! mainnet pools that cover the venue's shape space:
//!
//! - `5dDez…` — 4 tokens, equal weights, one Token-2022 mint, range manager on;
//! - `CSgrE…` — 9 tokens (72 directions), uneven weights, and token slot 6
//!   has a LIVE sell-off cap (10% / window) with a surge curve (80% → 25%),
//!   so the cap and conservative surge-free partial-fill limit run against
//!   the production program; full surge parity lives in coffer_fixtures;
//! - `BN4wp…` — 2 tokens, 80/20 weights;
//! - `AL4yx…` — 2 tokens whose LP-owned balance is tiny next to the virtual
//!   balance, so the upper bound is set by `AmountOutExceedsBalance`.
//!
//! Like the example suite, the tests SKIP when `SOLANA_RPC_URL` (and, for the
//! simulations, `programs/8iQt….so`) are absent.

mod common;

use common::SuiteConfig;
use solana_pubkey::{Pubkey, pubkey};
use titan_integration_template::coffer_venue::{COFFER_PROGRAM_ID, CofferVenue};

// Installs the allocation guard that powers the construction test's
// `assert_no_alloc` checks. The Makefile runs that test under `release-debug`
// so the guard is active; speed tests run under true `--release`.
#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Live mainnet pools the suite runs against, in order.
pub const POOLS: [Pubkey; 4] = [
    pubkey!("5dDezuaofYZUBdab8gSWY3ys86VbMrTxsRuf3ayqe8GJ"),
    pubkey!("CSgrEBxsghBsY1oXycBEhZVTd5PEbZuWFDZHHrtFF7yb"),
    pubkey!("BN4wpuvb4TyNbNigMW9cWYYrBfCH2NcquGp4AKxe8Lxm"),
    pubkey!("AL4yxA4o28c3dPwDSQm7VN1fxNeVfim13qzKkJfs4cKe"),
];

fn programs() -> Vec<Pubkey> {
    // The swap only CPIs into the token programs, which LiteSVM ships built in.
    vec![COFFER_PROGRAM_ID]
}

fn configs() -> Vec<SuiteConfig> {
    // `COFFER_TEST_POOL=<pubkey>` narrows the run to one pool.
    let only: Option<Pubkey> = std::env::var("COFFER_TEST_POOL")
        .ok()
        .and_then(|s| s.parse().ok());
    POOLS
        .iter()
        .filter(|p| only.is_none_or(|o| o == **p))
        .map(|pool| SuiteConfig {
            pool: *pool,
            programs: programs(),
        })
        .collect()
}

#[tokio::test]
async fn construction() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::construction::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn zero_input_spot_price() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::zero_input_spot_price::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn bound_simulation() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::bound_simulation::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn random_samples() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::random_samples::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn monotone() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::monotone::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn quoting_speed() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::quoting_speed::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn price_monotone() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::price_monotone::<CofferVenue>(&config).await;
    }
}

#[tokio::test]
async fn mean_value_theorem() {
    for config in configs() {
        eprintln!("--- pool {}", config.pool);
        common::mean_value_theorem::<CofferVenue>(&config).await;
    }
}

//! Your venue's test suite — the same shared assertions the example passes, run
//! against `YourVenue`. On a fresh template these are red (the `YourVenue`
//! methods are `todo!()`); implement `src/your_venue/mod.rs` and fill in the
//! config below to turn them green.
//!
//! Like the example suite, the tests SKIP when `SOLANA_RPC_URL` (and, for the
//! simulations, dumped program binaries) are absent.

mod common;

use common::SuiteConfig;
use solana_pubkey::Pubkey;
use titan_integration_template::your_venue::YourVenue;

// Installs the allocation guard that powers the construction test's
// `assert_no_alloc` checks. The Makefile runs that test under `release-debug`
// so the guard is active; speed tests run under true `--release`.
#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

fn pool() -> Pubkey {
    // FILL_IN: a real pool/market account for your venue to quote.
    todo!("set tests/your_venue.rs pool to a real pool or market account")
}

fn programs() -> Vec<Pubkey> {
    // FILL_IN: your program plus any runtime-dependency programs the swap CPI
    // touches. Dump each to programs/<id>.so via `make dump-programs`.
    todo!("set tests/your_venue.rs programs to your venue program dependencies")
}

fn config() -> SuiteConfig {
    SuiteConfig {
        pool: pool(),
        programs: programs(),
    }
}

#[tokio::test]
async fn construction() {
    common::construction::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn zero_input_spot_price() {
    common::zero_input_spot_price::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn bound_simulation() {
    common::bound_simulation::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn random_samples() {
    common::random_samples::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn monotone() {
    common::monotone::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn quoting_speed() {
    common::quoting_speed::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn price_monotone() {
    common::price_monotone::<YourVenue>(&config()).await;
}

#[tokio::test]
async fn mean_value_theorem() {
    common::mean_value_theorem::<YourVenue>(&config()).await;
}

//! Quay's venue test suite — the shared `tests/common` assertions run against
//! `QuayVenue` (the Quay `TradingVenue`), pointed at a live Titan-routed Quay
//! strategy on mainnet.
//!
//! The tests SKIP when `SOLANA_RPC_URL` (and, for the simulations, the dumped
//! Quay program binary) are absent.

mod common;

use common::SuiteConfig;
use solana_pubkey::{Pubkey, pubkey};
use titan_integration_template::quay::{QUAY_PROGRAM_ID, QuayVenue};

// Installs the allocation guard that powers the construction test's
// `assert_no_alloc` checks. The Makefile runs that test under `release-debug`
// so the guard is active; speed tests run under true `--release`.
#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// A live, Titan-routed Quay strategy on mainnet (`routing_flags & ROUTE_TITAN`).
/// One `Strategy` account is one pricing curve bound to a `(base, quote)` pair;
/// `QuayVenue::from_account` reads the program id off `account.owner`. Discover
/// others with `getProgramAccounts(QUAY_PROGRAM_ID)` filtered on the Strategy
/// discriminator (`0x03` at offset 0) — see `tests/quay_creation.rs`.
fn pool() -> Pubkey {
    pubkey!("5G1MkfhvpMxbw8PjnnnPNaZAFWYTGEZGLeWL3d7uF1e6")
}

/// The Quay program. SPL Token, Token-2022, and the System program are LiteSVM
/// builtins, so only Quay's own binary must be dumped to `programs/<id>.so`
/// (`make dump-programs`).
fn programs() -> Vec<Pubkey> {
    vec![QUAY_PROGRAM_ID]
}

fn config() -> SuiteConfig {
    SuiteConfig {
        pool: pool(),
        programs: programs(),
    }
}

#[tokio::test]
async fn construction() {
    common::construction::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn zero_input_spot_price() {
    common::zero_input_spot_price::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn bound_simulation() {
    common::bound_simulation::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn random_samples() {
    common::random_samples::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn monotone() {
    common::monotone::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn quoting_speed() {
    common::quoting_speed::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn price_monotone() {
    common::price_monotone::<QuayVenue>(&config()).await;
}

#[tokio::test]
async fn mean_value_theorem() {
    common::mean_value_theorem::<QuayVenue>(&config()).await;
}

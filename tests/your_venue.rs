//! Titan's unmodified shared acceptance assertions, wired to Ocelot.
//! With no RPC these SKIP. The curve assertions can also run through the local
//! fixture RPC; the original VM cannot load current Phoenix binaries.
//! Native conservative provider evidence lives in ocelot_phoenix/ocelot_replay.
#![allow(clippy::result_large_err)] // Preserve the upstream shared error type/assertions.

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
    std::env::var("OCELOT_TEST_MARKET")
        .unwrap_or_else(|_| "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM".into())
        .parse()
        .expect("valid test market")
}

fn programs() -> Vec<Pubkey> {
    // Current Phoenix needs the newer replay runtime in ocelot_replay.rs.
    vec![
        titan_integration_template::ocelot::PROGRAM_ID,
        titan_integration_template::ocelot::PHOENIX,
        titan_integration_template::ocelot::EMBER,
    ]
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

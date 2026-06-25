//! Reference suite: runs the shared venue tests against the worked Raydium AMM
//! example. This is the always-green baseline — if it fails, the template
//! itself is broken, not your integration. (Like the rest of the suite, the
//! tests SKIP when SOLANA_RPC_URL / program dumps are absent.)

mod common;

use common::SuiteConfig;
use solana_pubkey::{Pubkey, pubkey};
use titan_integration_template::example::{RAYDIUM_AMM_PROGRAM_ID, RaydiumAmmVenue};

// Installs the allocation guard that powers the construction test's
// `assert_no_alloc` checks. The Makefile runs that test under `release-debug`
// so the guard is active; speed tests run under true `--release`.
#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// Helper programs the Raydium AMM CPI touches inside the simulator.
const SPL_CALC_1: Pubkey = pubkey!("sspUE1vrh7xRoXxGsg7vR1zde2WdGtJRbyK9uRumBDy");
const SPL_CALC_2: Pubkey = pubkey!("ssmbu3KZxgonUtjEMCKspZzxvUQCxAFnyh1rcHUeEDo");

fn config() -> SuiteConfig {
    SuiteConfig {
        pool: pubkey!("Bzc9NZfMqkXR6fz1DBph7BDf9BroyEf6pnzESP7v5iiw"),
        programs: vec![RAYDIUM_AMM_PROGRAM_ID, SPL_CALC_1, SPL_CALC_2],
    }
}

#[tokio::test]
async fn construction() {
    common::construction::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn zero_input_spot_price() {
    common::zero_input_spot_price::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn bound_simulation() {
    common::bound_simulation::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn random_samples() {
    common::random_samples::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn monotone() {
    common::monotone::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn quoting_speed() {
    common::quoting_speed::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn price_monotone() {
    common::price_monotone::<RaydiumAmmVenue>(&config()).await;
}

#[tokio::test]
async fn mean_value_theorem() {
    common::mean_value_theorem::<RaydiumAmmVenue>(&config()).await;
}

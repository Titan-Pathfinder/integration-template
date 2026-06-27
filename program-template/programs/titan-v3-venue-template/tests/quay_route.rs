//! Quay's swap-route test — runs the shared end-to-end route suite against
//! `QuayVenue`. SKIPs cleanly unless `SOLANA_RPC_URL` is set and the route
//! program is built (`make build-program`).

mod common;

use common::{RouteConfig, run_swap_route};
use solana_pubkey::Pubkey;
use quay_titan_integration::quay::QuayVenue;

/// A live, Titan-routed Quay strategy on mainnet (same one `tests/quay.rs`
/// quotes against). The route program CPIs into Quay's `swap` to fill the leg.
fn pool() -> Pubkey {
    solana_pubkey::pubkey!("5G1MkfhvpMxbw8PjnnnPNaZAFWYTGEZGLeWL3d7uF1e6")
}

/// The Quay program. SPL Token, Token-2022, and the System program are LiteSVM
/// builtins, so only Quay's own binary must be dumped to `programs/<id>.so`.
fn venue_programs() -> Vec<Pubkey> {
    vec![quay_titan_integration::quay::QUAY_PROGRAM_ID]
}

#[tokio::test]
async fn swap_route_both_directions() {
    run_swap_route::<QuayVenue>(RouteConfig {
        pool: pool(),
        venue_programs: venue_programs(),
    })
    .await;
}

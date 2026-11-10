//! Coffer swap-route test — the same end-to-end suite the example
//! passes, run against `CofferVenue` through `swap_route_v3` in LiteSVM.
//! Runs every declared direction of each pool below; SKIPs cleanly without
//! SOLANA_RPC_URL / a fresh `make build-program`.

mod common;

use common::{run_swap_route, RouteConfig};
use solana_pubkey::{pubkey, Pubkey};
use titan_integration_template::coffer_venue::{CofferVenue, COFFER_PROGRAM_ID};

/// A 4-token pool with a Token-2022 mint, and a 2-token 80/20 pool.
const POOLS: [Pubkey; 2] = [
    pubkey!("5dDezuaofYZUBdab8gSWY3ys86VbMrTxsRuf3ayqe8GJ"),
    pubkey!("BN4wpuvb4TyNbNigMW9cWYYrBfCH2NcquGp4AKxe8Lxm"),
];

fn venue_programs() -> Vec<Pubkey> {
    vec![COFFER_PROGRAM_ID]
}

#[tokio::test]
async fn swap_route_all_directions() {
    for pool in POOLS {
        eprintln!("--- pool {pool}");
        run_swap_route::<CofferVenue>(RouteConfig {
            pool,
            venue_programs: venue_programs(),
        })
        .await;
    }
}

//! Reference swap-route test: runs the shared route suite against the worked
//! Raydium AMM example. SKIPs cleanly without SOLANA_RPC_URL / `make build-program`.

mod common;

use common::{RouteConfig, run_swap_route};
use solana_pubkey::pubkey;
use titan_integration_template::example::{RAYDIUM_AMM_PROGRAM_ID, RaydiumAmmVenue};

#[tokio::test]
async fn swap_route_both_directions() {
    run_swap_route::<RaydiumAmmVenue>(RouteConfig {
        pool: pubkey!("Bzc9NZfMqkXR6fz1DBph7BDf9BroyEf6pnzESP7v5iiw"),
        venue_programs: vec![RAYDIUM_AMM_PROGRAM_ID],
    })
    .await;
}

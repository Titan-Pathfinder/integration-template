//! Your venue's swap-route test — the same end-to-end suite the example passes,
//! run against `YourVenue`. Red once you've implemented YourVenue and pointed the
//! config below at a real pool + program (with SOLANA_RPC_URL set and the route
//! program built); SKIPs cleanly until then.

mod common;

use common::{RouteConfig, run_swap_route};
use solana_pubkey::Pubkey;
use titan_integration_template::your_venue::YourVenue;

fn pool() -> Pubkey {
    // FILL_IN: a real pool/market account for your venue to route through.
    todo!("set your_venue_route.rs pool to a real pool or market account")
}

fn venue_programs() -> Vec<Pubkey> {
    // FILL_IN: your venue's program(s) the swap CPI invokes.
    todo!("set your_venue_route.rs venue_programs to your route CPI dependencies")
}

#[tokio::test]
async fn swap_route_both_directions() {
    run_swap_route::<YourVenue>(RouteConfig {
        pool: pool(),
        venue_programs: venue_programs(),
    })
    .await;
}

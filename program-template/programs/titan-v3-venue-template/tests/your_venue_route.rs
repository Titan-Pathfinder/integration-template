//! Original shared router acceptance suite, wired to Ocelot.
//! Its LiteSVM 0.6.1 cannot load the captured Phoenix ELF. Modern-runtime
//! offline verification is in the host crate's tests/ocelot_replay.rs.

mod common;

use common::{run_swap_route, RouteConfig};
use solana_pubkey::Pubkey;
use titan_integration_template::your_venue::YourVenue;

fn pool() -> Pubkey {
    Pubkey::from_str_const("AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM")
}

fn venue_programs() -> Vec<Pubkey> {
    vec![
        titan_integration_template::ocelot::PROGRAM_ID,
        titan_integration_template::ocelot::PHOENIX,
        titan_integration_template::ocelot::EMBER,
    ]
}

#[tokio::test]
async fn swap_route_both_directions() {
    run_swap_route::<YourVenue>(RouteConfig {
        pool: pool(),
        venue_programs: venue_programs(),
    })
    .await;
}

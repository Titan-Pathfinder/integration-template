//! Guards that the route-builder `Venue` enum (in the off-chain crate's
//! `swap_route` module) and the program `Venue` enum (this crate's `state.rs`)
//! serialize to identical bytes.
//!
//! When you add your venue variant, add it to both enums in the same position
//! and to the `cases` list below.

use anchor_lang::AnchorSerialize;
use titan_integration_template::swap_route::Venue as RouteBuilderVenue;
use titan_v3_venue_template::state::Venue as ProgramVenue;

#[test]
fn venue_enum_matches_route_builder() {
    let cases = [
        (ProgramVenue::RaydiumAmm, RouteBuilderVenue::RaydiumAmm),
        (
            ProgramVenue::Ocelot {
                data: [0; 160],
                len: 148,
                quoted_input: 123,
            },
            RouteBuilderVenue::Ocelot {
                data: [0; 160],
                len: 148,
                quoted_input: 123,
            },
        ),
        (
            ProgramVenue::Ocelot {
                data: [255; 160],
                len: 112,
                quoted_input: u64::MAX,
            },
            RouteBuilderVenue::Ocelot {
                data: [255; 160],
                len: 112,
                quoted_input: u64::MAX,
            },
        ),
    ];

    for (program, route_builder) in cases {
        let program_bytes = program.try_to_vec().unwrap();
        let route_builder_bytes = route_builder.to_borsh_bytes();
        assert_eq!(
            program_bytes, route_builder_bytes,
            "Venue {program:?} serializes differently between program and route builder — the two \
             enums have drifted; check that variants match in name and order",
        );
    }
}

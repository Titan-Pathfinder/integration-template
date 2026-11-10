#![allow(clippy::result_large_err)] // `TradingVenueError` is large. Crate level because the type is used everywhere

pub mod account_caching;
// Verbatim contract port: never reformatted, never linted (see src/coffer/mod.rs).
#[rustfmt::skip]
#[allow(clippy::all)]
pub mod coffer;
pub mod coffer_venue;
pub mod example;
pub mod local_stand;
pub mod swap_route;
pub mod trading_venue;

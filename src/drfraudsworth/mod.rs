//! Dr. Fraudsworth TradingVenue implementation for Titan Pathfinder.
//!
//! Provides 6 venue instances covering 8 swap directions:
//! - 2 SOL pools (CRIME/SOL, FRAUD/SOL) via `SolPoolVenue`
//! - 4 vault conversions (CRIME<->PROFIT, FRAUD<->PROFIT) via `VaultVenue`

pub mod constants;
pub mod math;
pub mod state;
pub mod accounts;
pub mod instruction_data;
pub mod token_info_builder;
pub mod sol_pool_venue;
pub mod vault_venue;

pub use sol_pool_venue::SolPoolVenue;
pub use vault_venue::{VaultVenue, known_vault_venues, known_sol_pool_venues, all_venues};

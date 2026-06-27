//! Enumeration of supported pool/AMM protocol types.
//!
//! Each `TradingVenue` declares which protocol it implements (e.g. a specific
//! AMM, orderbook, or proprietary liquidity engine). Titan uses this enum to
//! label venues, group similar pools, and provide protocol-specific routing or
//! heuristics where applicable.

use std::fmt::Display;

/// Identifies the protocol family or implementation style of a trading venue.
///
/// Every AMM or custom pool that integrates with Titan must choose one of these
/// variants (or add their own) so the router and UI can correctly identify and
/// categorize the venue.
///
/// Protocols included here:
/// - `Quay`: Quay's on-chain DSL-priced market-maker strategies.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PoolProtocol {
    /// Quay market-maker strategies (DSL-priced curves on the Quay program).
    Quay,
}

impl Display for PoolProtocol {
    /// Display as a human-readable string.
    ///
    /// Delegates to the `From<PoolProtocol> for String` implementation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from(*self))
    }
}

impl From<PoolProtocol> for String {
    /// Convert a protocol enum into a canonical string representation.
    ///
    /// This is what will be used when Titan labels venues, logs activity, or
    /// exposes protocol metadata via API.
    fn from(protocol: PoolProtocol) -> Self {
        match protocol {
            PoolProtocol::Quay => "Quay".to_string(),
        }
    }
}

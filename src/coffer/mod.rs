//! Off-chain port of the `coffer-pool` swap math (Coffer `contracts`, branch `main`).
//!
//! Layout:
//! - `constants`, `math::*` — VERBATIM copies of the contract modules (only
//!   `use` lines changed). `tests/coffer_source_parity.rs` diffs them against
//!   the contract source (`COFFER_CONTRACT_SRC`, default path of the sibling
//!   checkout) so any upstream change is caught.
//! - `errors`, `state`, `prelude` — hand-written mirrors of the Anchor-only
//!   pieces (`#[error_code]`, `#[account]`, the prelude) with the same names
//!   and layouts.
//! - `swap` — the quote path: the swap handler's fee helpers and its
//!   guard / fee / curve / surge-fee segments, copied verbatim into a
//!   function so an exact-in quote reproduces `swap::handler` byte for byte.

pub mod constants;
pub mod errors;
pub mod math;
pub mod prelude;
pub mod state;
pub mod swap;

/// Lets the verbatim `constants.rs` resolve `anchor_lang::prelude::Pubkey`.
pub mod anchor_lang {
    pub mod prelude {
        pub use solana_pubkey::Pubkey;
    }
}

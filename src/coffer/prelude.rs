//! Stand-in for `anchor_lang::prelude::*` as used by the ported math.
//!
//! The contract's math modules only reach into the Anchor prelude for
//! `Result`, `require!` and `Pubkey`. This module supplies those three so the
//! ported bodies compile unchanged: `Result<T>` becomes a plain
//! `core::result::Result<T, ErrorCode>` (the contract's `Result` wraps an
//! `anchor_lang::error::Error` that every `ErrorCode` converts into, and every
//! `?` / `.into()` in the ported code type-checks identically against the
//! local alias).

pub use crate::coffer::errors::ErrorCode;
pub use solana_pubkey::Pubkey;

/// Mirrors `anchor_lang::Result<T>` for the ported modules.
pub type Result<T> = core::result::Result<T, ErrorCode>;

/// Mirrors Anchor's `require!(condition, ErrorCode::Variant)`: return the error
/// from the enclosing function when the condition is false. Re-exported below
/// so `use crate::coffer::prelude::*;` brings it into scope like Anchor's
/// prelude does.
macro_rules! require {
    ($cond:expr, $err:expr $(,)?) => {
        if !($cond) {
            return Err($err.into());
        }
    };
}
pub(crate) use require;

// PORTED VERBATIM from the Coffer contract's `src/math/mod.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! Pure math primitives used by the pool.
//!
//! - `fixed_point`  — 18-decimal helpers (mul/div with rounding direction,
//!   token-decimal scaling, Taylor-series `pow`).
//! - `log_exp_math` — Balancer V2's audited `ln`/`exp`/`pow` ported to
//!   `u128` / `i128`. The production swap and invariant paths use this.
//! - `weighted_math`— pool-level invariant + weight validation.
//! - `coffer_math`   — swap and BPT-mint/burn formulas.

pub mod fixed_point;
pub mod coffer_math;
pub mod log_exp_math;
pub mod weighted_math;
pub mod max_selloff;
pub mod surge_fee;

pub use fixed_point::*;
pub use coffer_math::*;
pub use log_exp_math::*;
pub use weighted_math::*;

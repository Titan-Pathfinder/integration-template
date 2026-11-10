// PORTED VERBATIM from the Coffer contract's `src/constants.rs` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
//! Compile-time constants shared across coffer-pool.
//!
//! ## Admin model
//! Two-level governance, both pubkeys live in storage (no hardcoded
//! addresses):
//!
//! - **Pool admin** — `CofferPool::pool_admin`, initialised to the wallet
//!   that called `initialize_coffer_pool`. Owns `set_swap_fee_rate` and
//!   `set_swaps_enabled`. Rotates via `initiate_pool_admin_transfer` →
//!   `accept_pool_admin_transfer`. Can be permanently disabled via
//!   `disable_pool_admin` (single-step, sets the field to
//!   `Pubkey::default()`).
//! - **Protocol admin** — `CofferPoolConfig::protocol_admin`, initialised
//!   when the config is created. Owns pool-pause, fee-rate updates,
//!   protocol-fee collection, emergency drain, and banned-extension
//!   maintenance. Rotates via `initiate_protocol_admin_transfer` →
//!   `accept_protocol_admin_transfer` (no zero-out path — keeping the
//!   protocol-level recovery channel always open).
//!
//! In production both keys point at PDAs owned by external admin
//! programs (e.g. the `protocol-admin` program's Treasury PDA), so admin
//! actions are reachable only via CPI from those programs.

use crate::coffer::anchor_lang;
// ─────────────────────────────────────────────────────────────────────────────
// Fixed-point precision
// ─────────────────────────────────────────────────────────────────────────────

/// 18-decimal fixed-point precision (1e18). All `LogExpMath` and
/// `FixedPoint` math use this scale for intermediate values.
pub const FIXED_POINT_PRECISION: u128 = 1_000_000_000_000_000_000;

/// Alias for `FIXED_POINT_PRECISION`. Reads more clearly inside math
/// expressions (e.g. `mul_div_down(x, y, ONE)`).
pub const ONE: u128 = FIXED_POINT_PRECISION;

// ─────────────────────────────────────────────────────────────────────────────
// Weight scale (basis points; 10_000 = 100%)
// ─────────────────────────────────────────────────────────────────────────────

/// Sum that pool weights must equal (10_000 = 100%).
pub const WEIGHT_SCALE: u64 = 10_000;

/// Convexity base `A` (in 18-decimal fixed point) for the per-token
/// variable sell-off surge fee curve:
///
/// ```text
///   fee(t) = slope_low + (slope_high - slope_low) * (A^t - 1) / (A - 1)
///   t      = (fill_ratio - threshold) / (PERCENT_SCALE - threshold)  ∈ [0, 1]
/// ```
///
/// `A = 4` gives a **moderately convex** curve (gentler than quadratic):
/// the fee stays low through most of the surge zone and climbs toward
/// `slope_high` near full window fill. Computed via `LogExpMath::pow`.
/// Tune convexity by changing only this constant (larger A ⇒ more convex).
pub const SURGE_FEE_CONVEXITY_FP: u128 = 4 * ONE;

/// Percentage scale shared by the relative (percent-of-value) parameters:
/// per-token `max_selloff_pct` and the range-manager change limits
/// (`range_manager_max_vb_change_pct` / `..._weight_change_pct`).
///
/// **10_000 = 100%**, so a stored value of `1000` means 10% and `500`
/// means 5%. Resolution is 0.01% (1 unit). A value of `0` disables the
/// associated check; values must be `<= PERCENT_SCALE`. Numerically this
/// is the basis-point scale, exposed as a percentage config.
pub const PERCENT_SCALE: u64 = 10_000;

/// Minimum per-token weight (1%). Prevents degenerate pools where one
/// token dominates the invariant by such a wide margin that
/// `LogExpMath::pow` saturates.
pub const MIN_WEIGHT: u64 = 100;

/// Maximum per-token weight (99%). Symmetric counterpart to
/// `MIN_WEIGHT` — keeps every pool with ≥ 2 tokens within `pow`'s
/// well-conditioned input range.
pub const MAX_WEIGHT: u64 = 9_900;

// ─────────────────────────────────────────────────────────────────────────────
// Fee scales
// ─────────────────────────────────────────────────────────────────────────────

/// Denominator for `swap_fee_rate` (1_000_000 = 100%, hundredths of bps).
pub const SWAP_FEE_PRECISION: u64 = 1_000_000;

/// Hard cap on `swap_fee_rate`. Capped at 10% (100_000) — anything
/// higher would let admins effectively confiscate trader value.
pub const MAX_SWAP_FEE_RATE: u32 = 100_000; // 10%

/// Denominator for `protocol_fee_rate` (10_000 = 100%, basis points).
pub const PROTOCOL_FEE_PRECISION: u64 = 10_000;

/// Hard cap on `protocol_fee_rate`. Capped at 50% (5_000) — anything
/// higher would let the protocol take every basis point of fee income,
/// starving LPs.
pub const MAX_PROTOCOL_FEE_RATE: u16 = 5_000; // 50%

/// Default `protocol_fee_rate` seeded onto every new pool when
/// `pool_initialize_config` doesn't override (20%).
pub const DEFAULT_PROTOCOL_FEE_RATE: u16 = 2_000;

// ─────────────────────────────────────────────────────────────────────────────
// Pool shape limits
// ─────────────────────────────────────────────────────────────────────────────

/// Minimum token count per pool.
pub const MIN_TOKENS: usize = 2;

/// Maximum token count per pool. Bounded by:
/// - per-tx account-list size on Solana legacy txs,
/// - `INIT_SPACE` of `CofferPool` (10 × per-token slots),
/// - the cost of running `LogExpMath::pow` 10 times in `add_liquidity`.
pub const MAX_TOKENS: usize = 10;

/// BPT mint decimals. Fixed at 9 to match the most common SOL/SPL
/// convention; the exact value is mostly cosmetic since BPT is internal.
pub const BPT_DECIMALS: u8 = 9;

/// Hard cap on a pool token's mint decimals, enforced at
/// `initialize_coffer_pool`.
///
/// `WeightedMath::calculate_invariant` normalises balances to 6 decimals by
/// dividing by `10u128.pow(decimals - 6)`. That exponentiation is unguarded,
/// so a mint with `decimals >= 45` would overflow the `u128` power. 18 is the
/// widest any real SPL mint uses and leaves the exponent at most 12.
pub const MAX_TOKEN_DECIMALS: u8 = 18;

// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds
// ─────────────────────────────────────────────────────────────────────────────

/// PDA seed prefix for pool accounts. Full seeds:
/// `[COFFER_POOL_SEED, config_pubkey, pool_id_le_bytes]`.
pub const COFFER_POOL_SEED: &[u8] = &[99u8, 117, 98, 105, 99, 95, 112, 111, 111, 108];

/// Program ID of the external `protocol-admin` program. The Treasury PDA
/// is `[b"treasury"]` under this program; Anchor's `seeds::program`
/// constraint derives it at runtime — see `initialize_config`. Only this
/// one program ID is hardcoded; the PDA itself is never spelled out in
/// the binary.
pub const PROTOCOL_ADMIN_PROGRAM_ID: anchor_lang::prelude::Pubkey =
    anchor_lang::prelude::Pubkey::from_str_const(
        "3jiojHZbjJQ7QLMGSTjFwxVEmx4NtuRy34nLAmsJME81",
    );

/// Seed of the `Treasury` PDA inside the protocol-admin program. Kept
/// here so coffer-pool can derive the Treasury PDA at runtime without
/// depending on the protocol-admin crate (which would create a circular
/// Cargo dependency, since protocol-admin already depends on coffer-pool
/// for CPI).
pub const PROTOCOL_ADMIN_TREASURY_SEED: &[u8] = b"treasury";

/// PDA seed prefix for BPT mint accounts. Full seeds:
/// `[BPT_MINT_SEED, pool_pubkey]`.
pub const BPT_MINT_SEED: &[u8] = b"bpt_mint";

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a basis-point weight (1e4 scale) to 18-decimal fixed-point.
///
/// # Parameters
/// * `weight_bps` — 0..=10_000
///
/// # Returns
/// `weight_bps * ONE / WEIGHT_SCALE`, the value `LogExpMath::pow` expects
/// as the exponent.
///
/// # Used by
/// `WeightedMath::calculate_invariant`, `CofferMath::calc_out_given_in`.
pub fn weight_to_fixed_point(weight_bps: u64) -> u128 {
    (weight_bps as u128) * ONE / (WEIGHT_SCALE as u128)
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc protocol parameters
// ─────────────────────────────────────────────────────────────────────────────

/// Floor on the BPT total supply that the pool must always preserve.
/// Enforced two ways:
/// 1. First deposit must mint at least this much (else
///    `InitialLiquidityTooSmall`).
/// 2. `remove_liquidity` caps the burn at `total_supply -
///    MINIMUM_INITIAL_BPT` and refunds the excess BPT to the user, so the
///    pool can never drain to zero supply (which would brick the
///    proportional-join math).
///
/// 1_000 = 0.000001 BPT (with 9 decimals).
pub const MINIMUM_INITIAL_BPT: u64 = 1_000;

/// Token-2022 extensions that **no pool creator may un-ban**, whatever
/// bitmap they pass to `initialize_coffer_pool`.
///
/// The per-pool override exists so a creator can choose their own token
/// policy, but the ban list is only a default — `Some(0)` un-bans
/// everything. What remains here is the single case the AMM cannot satisfy
/// at all, so it is an invariant rather than a default:
///
/// - `NonTransferable` (9) — the token cannot move at all, so it can neither
///   be seeded into a vault nor ever paid out. An AMM position in it is
///   unsatisfiable, not merely risky.
///
/// OR-ed into the effective bitmap, so the mask can only ever add bans.
///
/// Deliberately minimal: it contains only what the AMM cannot satisfy *by
/// construction*, not what an issuer could abuse. Issuer power — seizure,
/// pausing, re-pricing — is a trust decision about a named issuer, and this
/// protocol puts that with the pool creator plus disclosure, not in a
/// protocol-wide ban. A wider floor would have excluded the entire
/// regulated-asset segment (PYUSD, USDG, AUSD, PAXG, tokenised equities,
/// Ondo) while protecting nothing that is actually provable.
///
/// The protocol's real un-overridable safety does NOT live in this bitmap —
/// it lives in the unconditional value checks in
/// `util::token::check_mint_extensions`: unknown extension types, a frozen
/// `DefaultAccountState`, the Token-2022 native mint, and an ARMED transfer
/// hook. Those are exact conditions; a bitmap bit is only a presence test.
pub const HARD_BANNED_EXTENSIONS: u64 =
    1 << 9; // NonTransferable — a token that cannot move cannot be pooled

/// Default `banned_extensions` bitmap applied to every new
/// `CofferPoolConfig`. Bans extensions that break AMM accounting or
/// fungibility. Operators can override via `set_banned_extensions`.
/// The default is deliberately CONSERVATIVE even though the floor is
/// minimal: a creator who thinks about nothing gets the safe pool, and one
/// who wants to list an exotic asset has to clear a bit on purpose — which
/// is recorded on the pool and emitted, so LPs can see the policy.
pub const DEFAULT_BANNED_EXTENSIONS: u64 =
    (1 << 1)    // TransferFeeConfig — issuer can raise the fee to 100%
    | (1 << 3)  // MintCloseAuthority — close+reinit can swap the rules out
    | (1 << 10) // InterestBearingConfig — value drifts, pool prices raw
    | (1 << 12) // PermanentDelegate — issuer can seize from the vault
    | (1 << 14) // TransferHook — issuer can arm a hook after listing
    | (1 << 25) // ScaledUiAmount — value re-prices in discrete jumps
    | (1 << 26); // Pausable — issuer can halt the whole pool
// NOTE: ConfidentialTransferMint (4) is intentionally NOT banned. A vault
// can never hold a confidential balance — every confidential operation
// requires the ACCOUNT OWNER's signature, and the pool PDA only ever signs
// `transfer_checked`. Banning it bought no safety and excluded most of the
// regulated T22 segment. NonTransferable (9) is likewise absent here: it is
// rejected unconditionally in `check_mint_extensions`, so a bitmap bit for
// it would be redundant.

/// How many pieces the surge fee's taxed span is split into before charging.
///
/// This is NOT about the shape of the rate curve — that is exact. It is about
/// the mismatch between two spaces: the max-selloff window advances in INPUT
/// units while the fee is charged on OUTPUT, and the AMM curve is concave, so
/// one slice of window is not a proportional slice of output. Charging the
/// whole output at a single average rate therefore left audit M-05's
/// split-sell dodge partly open. Segmenting means computing each piece's REAL
/// output and charging it at that piece's own rate.
///
/// Measured whole-vs-split spread, by `weight_in / weight_out` and by the
/// curve's threshold (a lower threshold widens the taxed span, so a fixed
/// segment count resolves it more coarsely):
///
/// | threshold | ratio | no segmenting | 1     | 2     | 4     |
/// |-----------|-------|---------------|-------|-------|-------|
/// | 80%       | 50/50 |         1.88x | 1.045 | 1.011 | 1.003 |
/// | 80%       | 80/20 |         6.41x | 1.120 | 1.028 | 1.007 |
/// | 80%       | 90/10 |        81.8x  | 1.266 | 1.060 | 1.015 |
/// |  0%       | 80/20 |             — | 2.404 | 1.317 | 1.086 |
///
/// **4 is the measured ceiling, not a preference.** Each segment costs one
/// `CofferMath::calc_out_given_in`. On a live validator with the default
/// 200_000 CU budget: 4 segments passes the whole max-selloff suite, 6 starts
/// failing, and 8 aborts outright with `exceeded CUs meter`. A swap has to fit
/// the default budget — requiring every client to prepend a
/// `SetComputeUnitLimit` to ordinary swaps would not be an acceptable price.
///
/// This only became affordable when the rate curve went piecewise-linear. The
/// old convex form spent two `LogExpMath::pow` calls per segment just to
/// integrate the rate, and capped the scheme at ONE segment; the trapezoid
/// integral of a straight line costs nothing, and the freed budget is what
/// buys the other three.
///
/// The practical consequence worth knowing: at 4 segments even a threshold of
/// **0** stays inside 1.09, so there is no reason to forbid low thresholds and
/// no minimum-threshold rule in `set_max_selloff`.
///
/// Whatever residual remains, its DIRECTION is fixed: within a segment the
/// rate rises while marginal output falls, so the two are negatively
/// correlated and Chebyshev's sum inequality gives
/// `mean(rate · dy) <= mean(rate) · mean(dy)`. The charge can over-collect but
/// never fall below the exact output-weighted integral.
pub const SURGE_FEE_SEGMENTS: u8 = 4;

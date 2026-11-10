# Titan AMM Integration Template

A reference implementation and test suite for adding AMMs, CLMMs, and proprietary liquidity engines to Titan’s routing layer.

## Overview

Titan aggregates liquidity from heterogeneous venues (AMMs, CLMMs, orderbooks, proprietary pools) under a single unified quoting and routing interface.

This repository provides:

- A compact `TradingVenue` template for describing quote math, token metadata, account loading, and swap instruction shape
- A robust boundary-search engine for computing safe swap-size ranges
- Token metadata utilities, including Token-2022 support
- A caching abstraction for efficient on-chain account loading
- Simulation tests using LiteSVM ensuring off-chain quotes match on-chain execution
- Pricing tests ensuring the reported marginal price is consistent with the quoted output
- A fully worked Raydium example implementation

This template is the starting point for integrating your AMM into Titan.

## On-Chain CPI Template

This repo also includes `program-template/`, an Anchor template for the venue CPI
adapter Titan's router program calls during routed swaps.

Use it to verify your venue's on-chain swap instruction shape against Titan
router account layout and TitanPDA custody:

```bash
cargo check --manifest-path program-template/Cargo.toml
make build-program
```

The program template includes a real Raydium AMM CPI example plus a minimal
`venues/template.rs` file showing the common venue adapter shape.

## Core Components

- `TradingVenue`: implement account parsing, state refresh, token metadata,
  protocol labeling, exact-in quote math, and swap instruction construction.
- `QuoteRequest` / `QuoteResult`: Titan routes `ExactIn` only. All amounts and
  prices use raw atom units, not UI decimal scaling.
- `QuoteResult::price`: report the marginal derivative
  `d(output_atoms) / d(input_atoms)`. It must be positive, non-increasing, and
  consistent with `expected_output`.
- `bounds`: finds safe input ranges from a zero-input-safe `quote()`.
- `TokenInfo`: covers SPL Token, Token-2022, and transfer fee metadata. Do not
  duplicate transfer-fee handling in quote math.
- `AccountsCache`: loads required on-chain accounts with RPC caching.

## Included Tests

Every venue must pass the same shared suite in `tests/common/mod.rs`, run through
`tests/example.rs` for the Raydium reference and `tests/your_venue.rs` for your
integration.

- Construction and boundaries: deserialization, state loading, token info,
  boundary quotes, and no heap allocation inside `quote()`.
- Simulation: LiteSVM swaps compare on-chain output to off-chain `quote()` at
  boundaries and random samples, while checking accounts, monotonicity, and
  quote speed.
- Pricing: `price` must be positive, non-increasing, and bracket the realized
  average rate: `price(b) <= (f(b) - f(a)) / (b - a) <= price(a)`. The tests
  include atom-rounding slack for truncated integer outputs.

## Implementing Your Own Venue

Fill in the skeleton at **`src/your_venue/mod.rs`**, then wire the matching tests,
route builder, and program template files below. `program-template/...` means
`program-template/programs/titan-v3-venue-template`.

| Layer | File | Function / item | Update required |
| --- | --- | --- | --- |
| Creation parser | `src/your_venue/mod.rs` | `YOUR_PROGRAM_ID` | Replace with your venue's on-chain program id. |
| Creation parser | `src/your_venue/mod.rs` | `parse_pool_creations()` | Detect real pool-creation instructions and return `PoolCreation { protocol, pool, mints }`. |
| Creation parser | `tests/your_venue_creation.rs` | constants + `your_venue_pool_creation()` | Add a no-RPC fixture for one real pool-creation instruction. |
| Quote layer | `src/trading_venue/protocol.rs` | `PoolProtocol::YourPoolProtocol` | Rename or replace with your real protocol variant and display string. |
| Quote layer | `src/your_venue/mod.rs` | `YourVenue` fields | Add the pool state your quote math needs. |
| Quote layer | `src/your_venue/mod.rs` | `FromAccount::from_account()` | Deserialize the pool account and record state accounts to refresh. |
| Quote layer | `src/your_venue/mod.rs` | `protocol()` | Return your real `PoolProtocol` variant. |
| Quote layer | `src/your_venue/mod.rs` | `update_state()` | Fetch accounts through `AccountsCache`, deserialize live state, populate `token_info`, and initialize the venue. |
| Quote layer | `src/your_venue/mod.rs` | `quote()` | Implement exact-in quote math, including raw-atom marginal price. |
| Quote layer | `src/your_venue/mod.rs` | `generate_swap_instruction()` | Build your venue's swap instruction with the same per-leg `AccountMeta` shape the route builder will pass through. |
| Quote layer | `tests/your_venue.rs` | `pool()` + `programs()` | Point the shared off-chain suite at a real pool and required program binaries. |
| Route builder | `src/swap_route/mod.rs` | `Venue` enum | Add your route-builder venue variant in the same position and shape as the program template enum. |
| Route builder | `src/swap_route/mod.rs` | `protocol_to_venue()` | Map your `PoolProtocol` to your route-builder `Venue`; include any CPI parameters the program template must pass to your adapter. |
| Program layer | `program-template/.../src/state.rs` | `Venue` enum | Add the matching program-template venue variant, including any CPI parameters your adapter needs. |
| Program layer | `program-template/.../src/instructions/venues/template.rs` | `swap(<venue fields>, amount_in, account_metas)` | Replace this template with your venue CPI adapter: set program id, discriminator, and exact-in serialization. If renamed, remove the old placeholder so the scorecard no longer sees the default id. |
| Program layer | `program-template/.../src/instructions/venues/mod.rs` | `pub mod <venue>;` | Register your venue CPI adapter. |
| Program layer | `program-template/.../src/instructions/swap_route_v3.rs` | `perform_cpi_swap()` | Dispatch your `Venue` variant to your venue CPI adapter. |
| Program layer | `program-template/.../tests/venue_parity.rs` | parity cases | Add cases proving the route-builder and program-template venue enums serialize identically. |
| Program layer | `program-template/.../tests/your_venue_route.rs` | `pool()` + `venue_programs()` | Point the route simulation at a real pool and CPI program dependencies. |

If your swap CPI touches additional runtime programs, include them in
`program_dependencies()` and in the test program lists above.

### Running the tests

```bash
make build-program   # build the Titan router program template
make check-structure # fast no-RPC sanity checks
make test-example   # the Raydium reference suite — always green
make test-venue     # YOUR venue's suite (red until you implement YourVenue)
make scorecard      # print the integration scorecard only
make dump-programs  # fetch the program binaries the simulation tests load
```

The example and your venue run the *same* shared suite, so both are held to the
same bar. Everything runs cleanly on a fresh clone: the construction, simulation,
and pricing tests need a mainnet RPC endpoint (and, for the simulations, dumped
program binaries), so they **SKIP with an explanation** instead of failing when
those prerequisites are absent. To run them for real:

```bash
export SOLANA_RPC_URL=https://...   # a mainnet RPC endpoint
make build-program                  # rebuild the Titan router program template
make dump-programs                  # one-time: dump the venue programs into programs/
make test-example
make test-venue
```

`make check-structure` runs unit tests, scorecard assertions, and `Venue` enum
parity checks without requiring RPC.

`make scorecard` prints both scorecard sections: the *Example* section is your
always-green baseline (all four layers wired), and the *Your venue* section
tracks which placeholders you've replaced across the creation parser, quote,
program, and route-builder layers.

If your venue passes the suite, your quote() logic is sufficient to be assessed by
our team and go through the next stages of integration.

## Tips for Integrators
1. Always support zero-input quoting
2. Keep your deserialization strictly defensive, never panic
3. Don’t perform I/O, allocate heap memory, or panic inside quote()
4. A quote must average under 1 microsecond (1µs) — see the quoting_speed test
5. Make sure your instruction accounts match the program’s expectations
6. Report a marginal `price` in raw output atoms per raw input atom — positive, non-increasing in size, and consistent with `expected_output`

## Coffer venue

This fork implements the template for **Coffer / Cube DEX**
(mainnet program `8iQtGj9mcUfFUGaiCpPy89swC3s8YTC8FhVZWfgeZhwu`, contract
branch `main`): a 2..=10-token weighted AMM with virtual liquidity, a
ceil-rounded swap fee with a protocol cut, a per-token sliding-window sell-off
cap and a surge fee that rises as that window fills.

### Naming

The product is Coffer; the on-chain program crate still carries its earlier
internal name. This repository uses Coffer names everywhere. Wire
compatibility is untouched: every Anchor discriminator is a hard-coded byte
constant (`src/coffer_venue/instruction.rs`, pinned by a unit test to the
sha256 digest of its wire name, given as hex), the pool PDA seed keeps its
bytes as a byte-array literal, and the verbatim port is generated from the
contract source through a deterministic rename map
(`scripts/port_coffer_math.py`; the same map is applied by
`tests/coffer_source_parity.rs` before comparing).

### What is implemented

| Layer | Where |
| --- | --- |
| Verbatim math port | `src/coffer/{constants,math/*}.rs` are byte-for-byte copies of the contract modules (only `use` lines differ); `src/coffer/swap.rs` embeds the swap handler's guard / fee / curve / surge-fee / payout / state-update segments verbatim; `src/coffer/{state,errors,prelude}.rs` mirror the Anchor-only pieces. `tests/coffer_source_parity.rs` diffs all of it against the contract source and fails on drift. Regenerate with `python3 scripts/port_coffer_math.py [<Coffer src dir>]`. The `coffer` module is `#[rustfmt::skip]` for that reason. |
| Quote layer | `src/coffer_venue/mod.rs` — `CofferVenue` (`PoolProtocol::Coffer`, displayed as `"Coffer"`), `parse_pool_creations`, `update_state` (pool + mints + vaults + Clock sysvar), `directions_num`, `quote`, `generate_swap_instruction`, `AddressLookupTableTrait`. `price.rs` is the closed-form marginal price; `instruction.rs` the `swap` builder. |
| Route builder | `Venue::Coffer { token_in_index, token_out_index }` in `src/swap_route/mod.rs`. |
| Program layer | `program-template/.../venues/coffer.rs` CPI adapter, dispatch in `swap_route_v3.rs`, parity cases in `tests/venue_parity.rs`, route simulation in `tests/your_venue_route.rs`. |
| Production bytecode | `programs/8iQtGj9mcUfFUGaiCpPy89swC3s8YTC8FhVZWfgeZhwu.so` (sha256 `5128c578…`, committed, pinned by the fixture suite) so the offline fixtures run the exact mainnet build. |

Quote semantics: disabled pools/swaps, an inactive input token, or a frozen
input/output SPL vault cannot trade. `amount == 0` returns a zero-output
sentinel on otherwise available directions. Window, LP-balance and routing
limits produce a **partial fill** (`not_enough_liquidity = true`, `amount` =
the supported input, `expected_output` = exact output at that amount). The
caller must request that returned amount when building the executed swap;
`generate_swap_instruction` preserves its input request, as Titan requires.
`minimum_amount_out` is 0. The local router template does not enforce a
route-level minimum output. Production integration must separately confirm
where the user's minimum received amount is enforced on-chain; output parity
tests do not validate that protection.

**Surge routing policy:** the exact contract port still calculates the full
four-segment surge fee, but Titan quotes only the prefix where the contract
charges zero surge. This replaces the net-output-peak heuristic: real downward
output steps can occur before that peak, so a positive marginal price cannot
honestly represent the full curve. With `cap` and `before` resolved by the
contract's window transition at the refreshed Clock, the additional gross
input limit is

```text
max(0, floor((cap * (threshold + 1) - 1) / 10000) - before)
```

for a nonzero cap and an enabled surge curve with `threshold < 10000`.
This includes the last zero-fee integer fill cell. Cap zero permits no fill;
disabled caps/curves and threshold 10000 impose no additional surge limit.
The LP-balance limit can reduce the amount further. Beyond this prefix the
adapter returns a partial fill; once the window is already in the surge zone,
positive requests return zero fill and `bounds` has no tradable range. The
reverse direction remains independently evaluated. `price` is the smooth
weighted-curve derivative on this admitted prefix, with the ordinary input
fee accounted for; it does not pretend to differentiate the surge segments.
Full surge routing needs a compatible quote convention from Titan or a
contract change. No on-chain code is changed by this policy.

Every update reloads pool, mints, vaults and Clock and rebuilds the limits and
curve parameters. A failed update invalidates quoting until a complete update
succeeds. SPL Token and Token-2022 vault freeze is checked independently of
the Coffer `is_active` flag: the former blocks either side of the frozen vault,
whereas `is_active = false` only blocks selling that token. These checks cover
pool vaults; user token accounts are not part of a venue snapshot.

### Test tiers

```bash
# 1. no RPC: unit tests, verbatim-port parity, scorecard, enum parity
make check-structure
cargo test --release --test coffer_source_parity   # needs ../contracts or COFFER_CONTRACT_SRC=<program crate>/src

# 2. no RPC: routing policy, refresh/freeze and exact pinned-ELF parity
make test-coffer-offline

# 3. RPC-gated: the shared suite on live pools, and the route program
export SOLANA_RPC_URL=https://...
make build-program && make dump-programs
make test-example
make test-venue            # COFFER_TEST_POOL=<pubkey> narrows to one pool
cargo run --release --example mvt_diag   # per-direction MVT residual report
```

Tier 2 builds synthetic pools with the real layout, PDA seeds and ATAs, and
proves against the production program: exact payouts and exact post-swap pool
state on size grids; the exact `MaxSelloffExceeded` boundary (largest passing
input = the reported partial-fill amount); split-vs-whole sequences with a
state refresh between legs; window rotation after one and two periods incl.
the carry-over rescale when the snapshot changes; five surge-curve shapes
(kinked, no kink, threshold 0, near-100% threshold, 20/80 weights); a zero-fee
pool; dust inputs; output capped by the LP balance (protocol fees excluded);
deactivated input token / disabled pool / disabled swaps; a 10-token pool with
Token-2022 mints and two capped slots; the u64 balance ceiling.

Live pools used by tier 3 (`tests/your_venue.rs`): `5dDez…` (4 tokens, a
Token-2022 mint), `CSgrE…` (9 tokens; slot 6 has a LIVE 10% sell-off cap with an
80% → 25% surge curve), `BN4wp…` (80/20 weights), `AL4yx…` (LP balance far
below the virtual balance). The route simulation uses `5dDez…` and `BN4wp…`.

### Earlier measurements (before the conservative surge policy)

- Quote latency (Apple M-series, `--release`): ~0.2 µs for a plain quote,
  ~0.5 µs on a fixture grid, ~0.7 µs with the cap and surge fee active
  (`quote_latency_report`, `examples/quote_bench.rs`). The template's
  `quoting_speed` test passes on all four live pools.
- Exact on-chain parity: `bound_simulation` / `random_samples` pass on all four
  live pools (200 random sizes per direction, 88 directions) and the route
  simulation passes on every direction of both route pools.

### Known limitations

- **`mean_value_theorem` (shared suite) is red on two of the four live pools.**
  The contract rounds the swap fee UP in *input* atoms, so the exact output
  curve is a staircase with a flat step every `1/fee_rate` atoms. The shipped
  test allows two *output* atoms of slack per chord; on directions that pay
  more than ~2 output atoms per input atom (e.g. USDC → BONK, USDC → a
  12-decimal mint) one input atom of fee rounding on a chord of 40–250 atoms
  exceeds it. Measured: violations only at input sizes ≤ 110 809 atoms, worst
  5.7% (2%-fee pool, ~50-atom chord containing one fee step); no pointwise
  price can pass there (a chord with a step followed by one without forces
  `p(b) ≤ chord₁ < chord₂ ≤ p(b)`). With one input atom of slack —
  `atol = (2 + price_a) / (b - a)` — every one of the 88 directions passes with
  zero violations (`examples/mvt_diag.rs`). The Raydium reference is unaffected
  only because its output/input atom ratio is ~1.4.
- Full surge-fee swaps are intentionally outside the Titan routing domain.
  The previous peak/continuous-price approximation did not satisfy the
  pricing interface, including on inputs before the estimated peak. Tests
  reproduce the actual output declines against the pinned ELF and check that
  those inputs are excluded. Separate raw-port tests still exercise every
  surge shape, compare exact payouts and compare the complete post-swap pool
  account. Restricting surge does not remove the input-fee rounding limitation
  described above; the shared pricing tolerances have not been relaxed.
- The window arithmetic uses the Clock sysvar captured at `update_state` (the
  sysvar is in `get_required_pubkeys_for_update`, so it refreshes with the
  pool). Without a refresh between the quote and the landing block, the
  execution differs from the quote only through the window position, and only
  in one direction is that unsafe — see "Clock staleness" below.
- Overflow of the vault *token account* (`vault + amount_in > u64::MAX`) is not
  predicted from pool state alone; it needs more than `2^64 - vault` atoms.
- `parse_pool_creations` targets the `main` instruction shape (mints in
  `remaining_accounts`); pre-upgrade creations that passed a `tokens` vector
  are not recognised.
- `directions_num` is STRUCTURAL: every ordered pair of the pool's slots,
  regardless of the pool's mutable state. A deactivated input token, an
  exhausted sell-off window or a sidelined output token are answered by
  `quote` (contract error / zero fillable atoms) and by `bounds` (no quotable
  range), never by dropping the pair — see "Declared directions" below.

### Local-validator matrix (sell-off window + surge fee, end to end)

`scripts/local-stand/` brings up a `solana-test-validator` with the production
Coffer ELF at the mainnet program id, the Titan router built from
`program-template` at `T1TAN…`, and the live `CofferPoolConfig` (`E9K7…`,
`protocol_admin` rewritten to the local wallet so `set_pool_enabled` can be
exercised). `local-stand setup` then creates local mints (BONK-like 5 dec,
USDC-like 6 dec, SOL-like 9 dec, a metadata-only Token-2022 mint), the
TitanPDA, and one real pool per configuration below — created, seeded and
configured with the local wallet as pool admin — and writes
`scripts/local-stand/stand.json`.

```bash
make build-program                 # router ELF for the validator
scripts/local-stand/up.sh          # validator + mints + 32 pools (~2 min); prints the addresses
scripts/local-stand/run-matrix.sh  # tiers 1-5 in order, logs in scripts/local-stand/logs/
```

Tiers (`tests/local_stand_matrix.rs`, `program-template/.../tests/local_stand_route.rs`,
`tests/local_stand_range_manager.rs`; shared helpers in `tests/stand/mod.rs`):

1. Titan's shared suite (all eight tests) on every static pool, plus
   `construction` under the allocation guard;
2. the program-template route simulation (`swap_route_v3` in LiteSVM) on every
   static pool, every direction;
3. REAL `swap_route_v3` transactions through the router on the validator for
   every direction of every static pool at three sizes (lower bound,
   geometric middle, upper bound), comparing the user's ATA delta AND the
   post-swap pool account with `quote()` / `apply_swap`;
4. dynamic sequences on dedicated pools: chunked sells (direct and routed)
   until the window is full, the full window reported as unavailable while the
   program reverts with `MaxSelloffExceeded`, buy side unaffected, surge fee
   accrual in the output token's protocol bucket, the 100%-fee exhaustion
   point, real-time rotation on a 20-second window, and the
   `set_token_active` / `set_swaps_enabled` / `set_pool_enabled` toggles;
5. pool state changing UNDER the venue (`rm_*` pools): the range manager
   moving virtual balances and weights, the admin reconfiguring
   `set_max_selloff`, `add_liquidity` / `remove_liquidity`, and executions
   without a refresh across a window rotation — see "Range manager and admin
   changes" below.

Configuration matrix (BONK is slot 0 and capped; `(cap, period, threshold,
low/mid/high, kink)`; see `CASES` in `src/bin/local_stand.rs`):

| case | pool | config |
| --- | --- | --- |
| a_cap_only | 50/50 | cap 10%, surge off |
| b_thr0_kink50 | 50/50 | cap 10%, thr 0, 0/50/100%, kink 50 |
| c_thr80_nokink | 50/50 | cap 10%, thr 80%, 0/0/100% |
| d_thr80_full_fee | 50/50 | cap 10%, thr 80%, 100/100/100% |
| e_thr999_nokink | 50/50 | cap 10%, thr 99.9%, 0/50/100% (kink 99 with this threshold is rejected on-chain) |
| e2_thr9899_kink99 | 50/50 | cap 10%, thr 98.99%, 0/50/100%, kink 99 |
| f_flat | 50/50 | cap 10%, thr 50%, 5/5/5%, kink 60 (kink 50 is rejected: must exceed the threshold) |
| g_kink1 / g_kink99 | 50/50 | cap 10%, thr 0, 0/20/100%, kink 1 / 99 (kink 100 is rejected on-chain) |
| h_cap100 / h_cap1 | 50/50 | cap 100% / 1%, thr 50%, 0/25/50%, kink 75 |
| i_short_period | 50/50 | cap 10%, period 20 s, thr 50%, 0/20/50%, kink 80 |
| j_surge_no_cap | 50/50 | cap 0 with a surge curve configured (inert) |
| k_inactive | 50/50 | config b + `set_token_active(BONK, false)` |
| l_swaps_off / l_pool_off | 50/50 | config a + swaps disabled / pool disabled |
| w8020_b / w8020_c | 80/20 | configs b / c |
| m_four_token | SOL/BONK/USDC/MEME22 | BONK cap 10% thr 50% 0/10/30% kink 70; MEME22 (Token-2022) cap 5% thr 80% 0/0/50% |
| dyn_* | 50/50, 80/20 | dedicated copies of b, c, d, the 20 s pool and b for the sequence tests |
| rm_short | 50/50 | cap 10%, period 30 s, thr 50%, 0/20/50%, kink 80 — vb moves, rotation, stale execution |
| rm_w_surge / rm_w_cap | 50/50 | config b / cap only — weight moves |
| rm_lev | 50/50 | cap 10%, thr 50%, 0/10/30%, kink 70 — USDC vb pushed to 16× |
| rm_cfg / rm_liq | 50/50 | config b — `set_max_selloff` reconfiguration / liquidity add + remove |

Results of the last clean run (`scripts/local-stand/results/`, reproduced by
`up.sh && run-matrix.sh`; 32 pools, 4 mints):

These measurements predate the conservative surge routing policy. They
validate the contract port and the previous integration, not full-surge
availability in the current adapter. Contract-focused stand scenarios use
explicit raw-port parity; routing checks use the restricted venue quotes.
The current offline regressions are `coffer_fixtures` and `coffer_vault_freeze`.

| tier | result |
| --- | --- |
| 1 shared suite | 19 static pools × 8 tests: everything passes except `mean_value_theorem` on the 17 pools that have a USDC → BONK direction (see below); `construction` also passes under the allocation guard |
| 2 route simulation | 19/19 pools, every direction, 430 legs exact |
| 3 real routed txs | 126 `swap_route_v3` transactions on the validator, 126 exact against the quote AND the predicted pool account (one, on the 20 s pool, exact once the venue clock is set to the block time); `l_swaps_off` / `l_pool_off`: quote refuses, program reverts 6021 / 6020; `k_inactive`: BONK direction declared but unquotable, selling reverts 6054, buying exact; max 284 428 CU per routed tx with the surge active |
| 5 range manager / admin | see "Range manager and admin changes": every scenario exact against real transactions; the shared suite passes (except the documented `mean_value_theorem` residual) on each mutated pool |
| 4 dynamic | windows filled in chunks on b / c / 80-20 c (direct) and b (routed, 10 chunks crossing the kink) with every chunk exact; full window → direction still declared, `bounds` errors, quote reports 0 fillable, program reverts `MaxSelloffExceeded`, buy side exact; surge accrual in the USDC protocol bucket equals the sum of the predicted per-chunk fees (2 194 516 753 atoms on b); 100%-fee config: exhaustion point executed exactly, selling past it pays 0 on-chain; 20 s window: fill, rotate after one period (carry-over headroom sold exactly), fresh cap after two; deactivate / swaps-off / pool-off toggles behave as quoted and revert 6054 / 6021 / 6020 |

### Range manager and admin changes (state moving under the venue)

A pool's `pool_admin` and its appointed `range_manager` can rewrite, between
any two refreshes, everything a quote depends on: `range_manager_update`
moves `virtual_balance` and/or `normalized_weight` of chosen slots in one
instruction (bounded per step, compare-and-swap guarded, cooldown, optional
leverage band); `set_max_selloff` replaces the caps and surge curves;
`set_token_active` / `set_swaps_enabled` / `set_pool_enabled` flip kill
switches; `add_liquidity` / `remove_liquidity` scale balances AND the window.
Tier 5 (`tests/local_stand_range_manager.rs`) drives every one of these on the
validator and asserts exact parity after each move. The review conclusions:

- **State freshness.** `update_state` re-reads the pool account, every mint,
  every vault and the Clock sysvar on each refresh and rebuilds the per-direction curve
  parameters (`rebuild_directions`) from that fresh pool; nothing used by
  `quote` is cached across refreshes (no lazy fields; `Clone` is field-wise).
  The only value fixed at construction is the token set
  (`required_state_pubkeys`, `index_of`, `vault()`), which no instruction of
  the contract can change after `initialize_pool`; `update_state` rejects a
  pool whose mints or token programs differ from the ones it was built from.
  Refresh failure leaves the venue uninitialized until a complete snapshot is
  accepted; frozen vaults remain valid state and disable only affected pairs.
- **Declared directions.** `directions_num` returns every ordered pair of
  slots and nothing else. Titan documents it as the venue's *declared*
  directions, so a router may evaluate it once at registration; a filter on
  mutable state (input token deactivated, window exhausted, output slot
  unseeded) would then permanently drop a direction that was merely paused
  when the venue was registered. Declaring a paused direction costs nothing:
  `bounds` reports no quotable range, `quote` returns the contract's error
  (`TokenInactive`, `InactivePoolError`) or a partial fill of zero atoms, and
  the direction quotes again as soon as the state allows. (Titan's shared
  suite assumes every declared direction quotes; the stand runs it through
  `local_stand::QuotableDirections` on the one pool with a deactivated token
  and asserts the paused direction explicitly.)
- **Range manager and the sell-off window.** The quote runs the contract's own
  `check_and_advance` on a copy of the live row. After a
  `range_manager_update` the cap therefore stays `max_selloff_pct × old
  snapshot` (`selloff_vb_snapshot` is not rescaled by the manager — the
  accepted contract residual, audit L-14) and the surge fill is measured
  against that cap; at the next rotation the snapshot becomes the moved
  `virtual_balance` and the carry-over is rescaled by `new / old`, exactly as
  on-chain (asserted byte for byte on the post-swap account). A weight change
  leaves the window untouched (fill accounting is in input atoms) and changes
  the curve, the spot price (`w_in / w_out` enters the derivative) and the
  surge segment outputs; all are recomputed from the refreshed pool.
- **Surge domain.** The contract charges four moving segments of the taxed
  span. Their quantized average rates can produce downward output steps well
  before the maximum payout. The adapter therefore uses the exact zero-surge
  prefix described above, recalculated on every refresh, and one shared limit
  for full/partial quotes and `bounds`. There is no peak search or tolerance
  extension. `contract_output_declines_are_excluded_from_titan_quotes`
  reproduces both a broad decline and a sharp valid 1% kink in the pinned ELF;
  `zero_surge_domain_satisfies_pricing_invariants` checks the retained range
  with the shared price tolerance, isolating surge from input-fee rounding.
- **Clock staleness** (a quote at clock `t`, the transaction landing at
  `t' > t` with no refresh in between). The pool state being unchanged, only
  the window position differs: (a) inside a window the carry-over decays, the
  fill falls, the surge rate falls — the execution pays at least the quote;
  (b) across a rotation with the live `virtual_balance` at or above the
  snapshot (the usual case: selling the token raises its vb) the accumulated
  sells become a decaying carry-over on a larger cap — again at least the
  quote; (c) across a rotation with the live `virtual_balance` BELOW the
  snapshot — after buys of the token, an LP exit, or a range-manager cut — the
  new cap is smaller, the same input lands at a higher fill, and the execution
  pays LESS than quoted or reverts with `MaxSelloffExceeded`. Measured on the
  stand (tier 5, `rm_short`, 30 s window): (b) quoted 214 427 934, received
  337 101 206; (c) after the manager cut BONK's vb by 49%: quoted
  4 287 845 199, received 4 042 376 300 (5.7% less). In every case the venue
  math evaluated at the block time reproduces the execution exactly, i.e. the
  residual is purely the clock. Decision: the venue quotes the contract's
  arithmetic at the refreshed clock and does NOT pre-empt a rotation. A
  "min over now and the post-rotation state" policy would under-quote every
  capped direction of every pool for a horizon before each boundary (the
  post-rotation state is also unknowable exactly: the snapshot it takes is
  the live vb at landing time, which other swaps move), would break the
  exact-parity contract the suite checks, and would still not cover a
  transaction landing later than the horizon. Refreshing per slot reduces
  stale-state exposure but cannot guarantee the landing state. Keep the Clock
  sysvar (already in `get_required_pubkeys_for_update`) refreshed with the pool.
  A route-level minimum-output check is required to reject execution below
  the user's accepted output. That check is absent from this local router
  template; its presence in Titan's production transaction must be confirmed.
  Note that the template's `RpcClientCache` keeps
  positive entries forever, so a caller must `reset_cache` or use a fresh
  cache before each refresh, otherwise `update_state` re-decodes the same
  pool and clock.
- **Encapsulation.** `pool`, `now` and the per-direction parameters derived
  from them are private and only written together by `update_state`
  (`pool()`, `now()`, `epoch()`, `fill_limit()` are the read accessors), so
  no caller can desynchronise the cached curve parameters from the pool.

Findings from the matrix:

- A routed Coffer leg with the surge fee active does NOT fit the router's
  default 200 000 CU: the router spends ~45k CU creating the TitanPDA ATAs and
  the 4-segment surge charge costs the swap four extra curve evaluations
  (`<program> … consumed 123388 of 123388 compute units … exceeded CUs
  meter`). Measured per routed tx: 160–168k CU without surge, 250–285k with it.
  The LiteSVM harness simulates at 1.4M CU, which is why the route simulation
  never showed it. Tier 3/4 prepend `SetComputeUnitLimit(600_000)`; Titan's
  router client must budget accordingly.
- The sell-off window is a function of the clock. A quote at clock `t` and a
  transaction landing at block time `t' > t` differ when the window rotates in
  between (20 s pool) or when a non-zero carry-over decays with the surge
  active; tier 3/4 re-quote with the venue clock set to the block time and
  report those as "exact only at block time" (1 of 126 routed txs, on the
  20 s pool). Refresh the Clock together with the pool.
- With a curve that reaches 100% at full fill (b, c, g, e, w8020 …) the last
  atom(s) of the window cannot be sold for any output; the venue refuses them
  (zero fillable) while the program accepts them and pays 0. Selling past the
  100%-fee threshold of config d pays 0 as well; the venue reports the
  threshold as the fillable amount.
- The contract rejects a kink at or below the threshold and a kink of 100
  (`InvalidSurgeFeeConfig`): "threshold 99.9% with kink 99", "kink 100" and
  "threshold 50% with kink 50" are not deployable; the matrix uses the closest
  legal shapes (e, e2, f, g_kink99).
- `mean_value_theorem` fails on every pool with USDC → BONK (5 000 BONK atoms
  per USDC atom): the ceil'd input-side fee staircase, see above (worst 6.0e-3
  on the 80/20 pools, all at sizes ≤ ~2e5 USDC atoms). On the surge-active
  configs a second residual appears beyond the input-atom slack — the
  4-segment surge charge over-collects by an amount that varies with size:
  1.05e-4 (b), 1.21e-4 (g_kink99), 2.83e-5 (w8020_b) and 1.71e-3 on g_kink1
  (a kink at 1% makes the first segment's average rate the coarsest); zero on
  every other configuration.

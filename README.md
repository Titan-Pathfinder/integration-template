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

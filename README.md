# Quay × Titan Integration

Titan `TradingVenue` integration for the [Quay](https://quay.markets) program — a
DSL-priced, market-maker liquidity engine on Solana. It exposes each Quay
Strategy as a quotable venue for Titan's router and provides the on-chain CPI
adapter Titan calls during routed swaps.

## What's here

- **`src/quay/`** — the `QuayVenue` adapter: account loading (`FromAccount`),
  per-slot state refresh (`update_state`), exact-in quote math with a marginal
  price, swap-instruction construction, address-lookup-table keys, and
  pool-creation parsing for live discovery.
- **`src/trading_venue/`** — the Titan venue contract (`TradingVenue`,
  `QuoteRequest`/`QuoteResult`, `bounds`, `TokenInfo`, `PoolProtocol::Quay`).
- **`src/swap_route/`** — the off-chain route-leg builder and the `Venue` enum
  that mirrors the on-chain program.
- **`program-template/`** — the Anchor route program with the Quay venue CPI
  adapter (`instructions/venues/quay.rs`) Titan's router invokes.

One `QuayVenue` is one Strategy — one pricing curve on one `(base_mint,
quote_mint)` pair. Titan's router holds many (one per active strategy) and
aggregates quote distribution across them.

## How quoting works

- Titan routes `ExactIn` only. All amounts and prices are in raw atoms.
- `quote()` calls `quay_sdk::simulate::simulate_swap_in` (bit-identical to the
  on-chain `swap`) on a fixed-size stack buffer — **no heap allocation**, every
  curve, stateful or not.
- `QuoteResult::price` is the marginal rate `d(output)/d(input)`. Because the DSL
  curve is a black box, it's computed by finite difference with the step sized to
  a fixed output target (`2^20` atoms) so integer-atom quantization stays well
  under Titan's mean-value-theorem tolerance.
- A strategy is surfaced only when it opts into Titan (`routing_flags &
  ROUTE_TITAN`), its on-chain halt set is clear, and it has no transfer-fee mints.

## On-chain CPI adapter

`program-template/` is the Anchor route program. `swap_route_v3` takes custody
through the TitanPDA and CPIs into each leg's venue adapter;
`instructions/venues/quay.rs` builds Quay's `swap` instruction (`[disc 0x20,
amount_in u64, min_out=0 u64, side u8]`) and forwards the route-assembled account
metas. The off-chain `swap_route::Venue` and the on-chain `state::Venue` enums are
kept byte-identical by `tests/venue_parity.rs`.

```bash
make build-program   # anchor build of the route program
```

## Testing

```bash
make check-structure  # no-RPC: lib tests + scorecard + Venue enum parity
make scorecard        # print the integration scorecard
make test-venue       # the full Quay venue suite (needs RPC; see below)
make dump-programs     # dump the Quay program binary for the simulation tests
```

`make check-structure` runs on a fresh clone with no network. The full suite
(`make test-venue`) drives the shared assertions in `tests/common/mod.rs` against
a live Titan-routed Quay strategy and skips with an explanation when
`SOLANA_RPC_URL` (and, for the on-chain route test, `make build-program` +
`make dump-programs`) are absent:

```bash
export SOLANA_RPC_URL=https://...   # a mainnet RPC endpoint
make build-program
make dump-programs
make test-venue
```

The suite covers construction/boundaries, zero-input spot price, output
monotonicity, sub-1µs quote speed, price monotonicity and the mean-value
theorem, and on-chain parity — LiteSVM executes the adapter's own swap (and the
full route CPI) and asserts the fill equals `quote()`.

## Design notes

- **Zero-input safe** — Titan requests `amount == 0` quotes for the spot rate;
  `quote()` returns zero output with a positive `f'(0)`.
- **No panics, defensive decoding** — malformed accounts surface as errors.
- **Live clock** — `get_required_pubkeys_for_update()` includes the `Clock`
  sysvar so curves using `LoadNowSlot` / `LoadNowUnixSec` see the same numbers a
  real swap would, deduped across all Quay venues.

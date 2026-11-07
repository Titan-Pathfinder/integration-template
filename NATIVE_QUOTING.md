# Native quoting implementation, 7 November 2026

**The Ocelot issuer kernel is implemented; the native Phoenix matcher is still
missing. This is not an operational production quoter.** No production provider
is bundled, and a venue without one reports `initialized() == false` and rejects
`quote()` and generic transaction construction. There is no simulation fallback.

## Implemented path

`src/ocelot/quote.rs` performs checked integer arithmetic on prepared state:

1. Apply the issuer mint fee; size the Phoenix IOC in native lots. Redemption
   sizes use accounting supply and round up only on the final circulating exit.
2. Ask a native Phoenix provider for the full-fill execution notional, exact fee,
   post-order collateral and available withdrawal capacity.
3. Value that execution at Ocelot's original native mark. Price shares using
   equity after accrued streaming liabilities, not cached NAV.
4. For redemptions, charge adverse execution to the exiting holder, retain
   favorable execution on partial exits, allocate streaming liability, and apply
   the redemption fee. Full accounting-supply exits withdraw the closed position.
5. Enforce supply, deposit/equity caps, share rounding, global/market withdrawal
   capacity and finance margin after mint or withdrawal. Reject partial fills,
   changing funding, unavailable collateral and zero outputs.

The instruction builder uses the same lot-sizing function. Its generic entrypoint
uses the native quote's output; the reference VM cannot be called from this path.
Default library builds contain no LiteSVM dependency. The modern VM and
`reference_quote()` require `reference-simulation` plus an explicit
`load_reference_oracle(cache)` call. State refresh clears previous prepared quotes
and the reference VM; failed refreshes cannot serve an old quote.

## Phoenix provider contract

Install an `Arc<dyn PhoenixQuoteFactory>` using `set_phoenix_factory()`. Its
`prepare(&State)` method runs on installation and every account refresh, returning
an immutable `Arc<dyn PhoenixQuoter>` bound to that state. Preparation can decode
accounts and build cumulative depth structures. Each `PhoenixQuoter::quote(&Order)`
must use native arithmetic, with no simulation, network call or heap allocation.
The combined issuer and provider call must meet Titan's one-microsecond budget.

`Order` includes exact lots, side, reduce-only flag, IOC price limit and the exact
collateral deposit that precedes matching. `Fill` includes full matched lots,
executed quote atoms, actual fee atoms, post-order collateral, immediately
withdrawable/redeemable collateral and the resulting cumulative funding index.
No partial-input execution is supported. Provider preparation/execution must
validate its account bindings, supported deployed ABI, Phoenix market/trader
permissions, restart status, maker and taker risk, parent-to-child deposit path,
and Phoenix/Ember custody and withdrawal availability. It must reject unsupported
states; accepting raw prices with an assumed unlimited risk budget is invalid.

The captured state includes the FIFO book, spline account, perp asset map and
index/buffer arenas. A provider must subscribe to all required maker/portfolio
dependencies and clock updates, not just changes to the book. Expanded arena
groups currently fail closed.

## Why the public SDK is insufficient by itself

Phoenix documents a combined FIFO/spline matcher with risk caps, expiry, pro-rata
allocation, hidden take liquidity during uncrossing, and spline slide prices.
The public SDK's FIFO BBO explicitly excludes spline uncrossing, and its spline
geometry helper is not a complete risk-aware execution engine. In our SOL3L
snapshot, a raw FIFO ask at 9,010 ticks does not describe the 9,011-tick taker
price inferred from the captured state and exact output comparisons after uncrossing. A raw depth sum would quote the wrong output.

Primary references:

- [Phoenix matching engine](https://docs.phoenix.trade/phoenix/matching-engine/matching-engine)
- [Spline liquidity and risk/uncrossing rules](https://docs.phoenix.trade/phoenix/matching-engine/spline-liquidity)
- [SDK FIFO BBO limitation](https://github.com/Ellipsis-Labs/rise-public/blob/2ce47221ef04a8d5879d39d31e82f9e92021753e/rust/accounts/src/orderbook/mod.rs)

The next question for Phoenix, and Titan if it already supports Phoenix perps:

> Do you have a native Rust execution quoter or matching-engine library we can
> use from cached accounts, including spline uncrossing/slide prices, maker risk
> limits and exact taker fees? Titan requires the combined Ocelot quote under
> 1 microsecond with no simulation. We have the issuer math and replay fixtures;
> if there is no reusable library, can you provide the matching and risk-cap
> reference implementation and test vectors so we can implement it faithfully?

## Evidence and limits

`tests/ocelot_native.rs` independently checks the issuer math using a bounded,
TEST-ONLY fixed-price provider for two pinned snapshots. It contains neither a
general matcher nor production maker-risk/withdrawal validation. The fixture
provider rejects other market/slot pairs and orders above its test range.
Its prices are inputs, not output lookup tables derived from the issuer formula.

Replay comparisons cover both mint/redeem directions, deterministic random sizes,
adjacent lot boundaries and a full SOL2S exit: **126 exact output matches and one
matching post-withdrawal finance-margin rejection** against the captured deployed
programs. Additional tests cover allocation-free success/error paths, bootstrap,
cash-only exits, abandoned supply, cap/fee/collateral rejection, bucket refill,
and refresh invalidation. These checks establish issuer behavior for those
cases, not completeness across Phoenix states.

The benchmark deliberately reports **issuer plus fixture matcher only**, with
one million alternating mint/redeem calls. Its batch means are not per-call
latency percentiles, and its result cannot establish complete integration speed.
On the development Mac, the November 7 run measured batch-mean p50 **75.1 ns**
and p99 **107.3 ns**, leaving most of the budget for native Phoenix matching.
This is a local measurement, not a production latency guarantee. Run it with:

```sh
cargo +1.96.0 test --release --locked --test ocelot_native benchmark_issuer_with_fixture_matcher -- --ignored --nocapture
```

Lot steps are preserved. Following the owner's November 7 instruction, work
proceeds on the assumption Titan can handle them. `price` reports the local
within-lot slope with integer rounding relaxed; it is not a globally concave
surrogate, nor a derivative at a jump. Titan must confirm its routing convention.
The upstream acceptance assertions have not been relaxed or represented as green.

Before production: supply and validate the full native provider, compare many
fresh states including crossed splines/multiple levels/risk limits, benchmark the
combined path, then obtain Titan's routing/ABI acceptance and listing approval.

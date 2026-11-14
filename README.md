# Ocelot native issuer integration for Titan

**14 November: native midpoint fallback with a five-bp output discount.**
Normal books keep the existing conservative curve. Raw locked/crossed books
prepare a common-midpoint approximation, rounding the midpoint up for buys and
down for sells, then return `floor(curve_output * 9995 / 10000)` with the marginal
price scaled by the same factor. The extra discount is not applied to normal
books and is not an additional on-chain fee.

Both preparation and quoting use Titan's supplied cached accounts. Quotes perform
no network calls, VM execution or allocations. Hawkeye research is not part of
the runtime. The default library still has no LiteSVM dependency.

**The midpoint policy is approximate, not a guaranteed execution-output bound.**
Known constructed crossed states still overquote after the five-bp discount.
Native checks remain active, but their midpoint-based price/capacity estimates
are not a proof of exact Phoenix slide behavior. Titan's route-level user minimum
protects execution; it can reject a transaction if actual output is insufficient.
See [policy, counterexamples and current validation](MIDPOINT_VALIDATION.md).

This update is for Titan engineering review in
[PR #10](https://github.com/Titan-Pathfinder/integration-template/pull/10).
It is not Titan acceptance or activation. No Ocelot program/ABI changes are
included. ANT3L/ANT2S remain outside the validated oracle-policy profile.

## Earlier submission history

The following records the earlier submission and review history.
The teammate's final review of `a8c5f9fd` closed the last blocker and found no new
blockers for same-state quote correctness and execution safety. That reviewed base was
submitted for Titan engineering review in [upstream PR #10](https://github.com/Titan-Pathfinder/integration-template/pull/10)
at `ac06e77`, with [standalone export verification](https://github.com/Ocelot-fi/integration-template/blob/ac06e77b57b7c0c8634d27151d013ce74f5bedd3/EXPORT_VALIDATION.md).
Titan's error/timing and routing/runtime acceptance remain outstanding. Live
routing is not activated.

**Slippage follow-up:** the candidate now uses Titan's confirmed route-level
user minimum instead of a fixed 1% Ocelot floor. This adapter change follows the
earlier internal sign-off and requires Titan's production-router verification.
See [implementation and 120-transaction rollback evidence](SLIPPAGE_VALIDATION.md).
Titan subsequently agreed to removing the adapter's 20-second cutoff. The
required deadline field is now `i64::MAX`, adding no practical adapter expiry.

Before the curve update, the complete provider passed 205 captured-state execution comparisons without an
overquote or false acceptance; worst observed underquote was 4.144072 bp after
restricting maker risk to the reviewed oracle-policy profile (200/205 are within 1 bp). The review
hardening fixes mixed-read preparation, classifies all 39 fixture declines, and
adds actual-transaction expiry, cash, full-exit capacity and binding-maker tests.
The follow-up fixes divergent-oracle eligibility, pins price-validity policy,
retains subscriptions for absent dependencies, and verifies cold/warm bootstrap
after executed full exits. A further fix requires spot/perp validity even at zero
pricing weight. See [latest review evidence](NATIVE_QUOTING.md#zero-weight-follow-up-10-november). This is
sample evidence, not a universal error bound. Unsupported risk/price configurations
fail closed; the current crossed-book policy is described above. Gally's SDK link is not an implementation dependency.
The captured Anthropic oracle policy and special-price status are outside this
provider's supported scope; ANT3L/ANT2S require separate validation before activation.

`reference_quote()` remains an opt-in correctness oracle, behind the
`reference-simulation` feature and explicit `load_reference_oracle(cache)`.
Default library builds do not depend on LiteSVM.
Based on [Titan's template](https://github.com/Titan-Pathfinder/integration-template)
at `c0bb5c9b7f7fc7d00f80c24852ef36942fc8464d` (see `UPSTREAM.json`). The shared
quote and router acceptance assertions are retained. Their skipped tests do not
constitute integration acceptance. See [evidence and Titan questions](HANDOFF.md).

## Implemented

- Creation parser for Phoenix `init_market`, validated market/mint PDAs, checked
  account decoders, finance revision, native mark, signed position and fee accrual.
- Account discovery through `AccountsCache`, including nested Phoenix/Ember
  accounts, withdrawal throttles, clock and cluster restart slot. Programdata is
  loaded/subscribed only when the reference oracle is explicitly enabled.
  Absent requested accounts remain watched; zero supply still discovers the
  withdrawal queue and optional market throttle.
  A failed refresh invalidates prior prepared quotes and the execution oracle.
- `phoenix_mint_v3` and `phoenix_redeem_v3` with exact input, full-fill IOC,
  reduce-only redemption, minimum output, deadline and finance revision guards.
- Matching host/program `Venue::Ocelot` encoding and a CPI adapter that only
  accepts those v3 instructions. A changed runtime input fails instead of
  executing an order sized for a different quote.
- Native issuer math, shared lot sizing, prepared quote limits, and an explicit
  native Phoenix-provider interface and a bundled capacity-limited spline provider.
  Allocation checks and complete issuer-versus-execution comparisons cover both directions.
- Local replay of the actual captured Ocelot, Phoenix and Ember executables for
  SOL3L and SOL2S; direct versus routed output parity and guard rejection tests.
  Fixtures contain public chain state. Synthetic user balances and a local ALT
  make replay possible without a wallet, RPC submission, or mainnet funds.

`build_with_minimum()` requires an explicit positive output bound for standalone
execution. Following Titan's November 10 guidance, `generate_swap_instruction()`
encodes zero venue minimum and **must only be used inside a route that enforces
the user's final minimum**. Titan confirms its production router owns that check;
the supplied template router does not implement it. The adapter still validates
quote eligibility, exact input, finance revision and the 100 bps IOC price band.
Both adapter builders encode `i64::MAX` in the required v3 deadline field,
leaving transaction expiry to Titan/frontend/wallet and Solana. The on-chain
deadline comparison is unchanged; zero would mean expired, not disabled.
The IOC band remains separate from user slippage. See [slippage tests and production
acceptance request](SLIPPAGE_VALIDATION.md). The caller must preserve the
quoted input. Variable intermediate amounts and independently quoted repeated
visits to the same Ocelot market are not supported.

Current coverage is classic SPL USDC/LT tokens, finance-enabled Phoenix markets,
and one arena account per Phoenix index/buffer group. Expanded arena groups,
unknown fee/finance layouts, unsupported token formats, ExactOut, stale marks,
and malformed bindings fail closed. Market discovery is not a listing allowlist.

## Verify without RPC

Rust 1.96.0, Cargo lockfiles, and `cargo-build-sbf` 2.3.0 with platform tools v1.54
were used. Run these commands from this directory:

```sh
cargo +1.96.0 test --locked --lib --test ocelot --test ocelot_native --test your_venue_creation
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_native --test ocelot_replay --test ocelot_state
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_spline --test ocelot_phoenix
cargo +1.96.0 test --locked --test ocelot_slippage
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_curve
cargo +1.96.0 test --locked --manifest-path program-template/Cargo.toml --lib --test venue_parity
```

The first command exercises native issuer math without a VM. The second enables
the exact-execution and lot-boundary regressions. The routed test is explicitly ignored until you build the router:

```sh
(cd program-template && cargo-build-sbf --tools-version v1.54 --arch v0 --manifest-path programs/titan-v3-venue-template/Cargo.toml)
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_replay -- --ignored --nocapture
```

The final command tests all four bundled snapshots, larger sizes and full exits and fails if the router binary is
missing. Optional `OCELOT_SNAPSHOT` (directory or `.bin.gz`) and `OCELOT_MARKET`
override the fixture location; routed native comparisons use the bundled provider
and require a supported state. Use `probe` for fresh reference quotes. No deployment command is needed.

To verify atomic rollback with the local final-output guard model:

```sh
(cd program-template && cargo-build-sbf --tools-version v1.54 --arch v0 --manifest-path test-programs/output-guard/Cargo.toml)
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_slippage -- --include-ignored --nocapture
```

Build the router first as above. The guard is a separate **test-only** program,
excluded from the default workspace members. Never deploy it or use it to protect
real funds. This proves adapter behavior and Solana transaction rollback against
captured executables, not Titan's production minimum calculation or wiring.

The upstream RPC suites (`tests/your_venue.rs` and the program's
`tests/your_venue_route.rs`) remain diagnostic acceptance gates. The unchanged
quote-shape assertions now pass against four snapshots through a local fixture
RPC; see [commands and limits](CURVE_VALIDATION.md#reproduce). Their LiteSVM 0.6.1
cannot load the captured Phoenix EM_SBPF executable. Offline replay uses LiteSVM 0.16.0 while
preserving Titan's Solana 2 public types. Do not interpret an RPC-less `cargo test`
run, or the template's structural scorecard, as production acceptance.

## Capture fresh public state

Use a fresh directory and your read-only RPC URL; the example makes no write RPCs:

```sh
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com RUST_LOG=info \
  cargo +1.96.0 run --locked --features reference-simulation --example probe -- /tmp/ocelot-titan-capture \
  AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM --refresh
```

After discovery, `--refresh` captures discovered dependencies, including programdata,
in one `getMultipleAccounts` response and prints each ELF hash. The bundled
fixtures predate programdata subscription and captured executables separately,
as recorded in the verification notes. The
probe prints exact-size replays and adjacent lot boundaries, then writes
`accounts.bin.gz` and capture metadata. Reusing a folder without `--refresh`
replays its saved state. The archive is a gzip-compressed bincode
`BTreeMap<String, Vec<u8>>`; each key is a public address and each value encodes
`Option<solana_account::Account>`.

No Ocelot vault program source, production listing, fees, caps, throttles, service,
or live Titan deployment changes are part of this package.

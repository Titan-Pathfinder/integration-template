# Ocelot native issuer integration for Titan

**Status: conservative native Phoenix quoting implemented and installed by default
for supported cached states.** `TradingVenue::quote()` performs issuer arithmetic,
spline pricing and risk/withdrawal checks without simulation, network access or heap
allocation. See [implementation, supported scope and measurements](NATIVE_QUOTING.md).
The teammate's final review of `a8c5f9fd` closed the last blocker and found no new
blockers for same-state quote correctness and execution safety. The candidate is
submitted here for Titan engineering review.
Titan's error/timing and routing/runtime acceptance remain outstanding. Live
routing is not activated.

The complete provider passed 205 captured-state execution comparisons without an
overquote or false acceptance; worst observed underquote is now 4.144072 bp after
restricting maker risk to the reviewed oracle-policy profile (200/205 are within 1 bp). The review
hardening fixes mixed-read preparation, classifies all 39 fixture declines, and
adds actual-transaction expiry, cash, full-exit capacity and binding-maker tests.
The follow-up fixes divergent-oracle eligibility, pins price-validity policy,
retains subscriptions for absent dependencies, and verifies cold/warm bootstrap
after executed full exits. A further fix requires spot/perp validity even at zero
pricing weight. See [latest review evidence](NATIVE_QUOTING.md#zero-weight-follow-up-10-november). This is
sample evidence, not a universal error bound. Unsupported risk/price configurations
and crossed splines fail closed. Gally's SDK link is not an implementation dependency.
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

`build_with_minimum()` accepts an explicit output bound. The template's generic
`generate_swap_instruction()` uses a 1% output tolerance, a 20-second deadline,
and a 100 bps IOC price band. These are reference policy defaults to agree with
Titan, not an approved production slippage policy. The caller must preserve the
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

The upstream RPC suites (`tests/your_venue.rs` and the program's
`tests/your_venue_route.rs`) remain diagnostic acceptance gates. Their original monotone/concave assertions conflict with Phoenix lot steps, and
their LiteSVM 0.6.1 cannot load the captured Phoenix EM_SBPF executable. Offline replay uses LiteSVM 0.16.0 while
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

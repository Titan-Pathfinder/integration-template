# Ocelot native issuer integration for Titan

**Status: review candidate, not ready for production routing.** Market loading,
v3 transaction construction, the Titan template CPI adapter, and offline replay
are implemented. `TradingVenue::quote()` deliberately returns an error: Ocelot's
Phoenix lot boundaries violate the template's monotone/concave quote contract.
`reference_quote()` executes the deployed programs locally and is a correctness
reference, not Titan's required allocation-free, sub-microsecond quote engine.

Based on [Titan's template](https://github.com/Titan-Pathfinder/integration-template)
at `c0bb5c9b7f7fc7d00f80c24852ef36942fc8464d` (see `UPSTREAM.json`). The shared
quote and router acceptance assertions are retained. Their skipped tests do not
constitute integration acceptance. See [evidence and Titan questions](HANDOFF.md).

## Implemented

- Creation parser for Phoenix `init_market`, validated market/mint PDAs, checked
  account decoders, finance revision, native mark, signed position and fee accrual.
- Account discovery through `AccountsCache`, including nested Phoenix/Ember
  accounts, withdrawal throttles, clock, cluster restart slot, and programdata.
  A failed refresh invalidates the prior execution oracle.
- `phoenix_mint_v3` and `phoenix_redeem_v3` with exact input, full-fill IOC,
  reduce-only redemption, minimum output, deadline and finance revision guards.
- Matching host/program `Venue::Ocelot` encoding and a CPI adapter that only
  accepts those v3 instructions. A changed runtime input fails instead of
  executing an order sized for a different quote.
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
cargo +1.96.0 test --locked --lib --test ocelot --test your_venue_creation --test ocelot_replay
cargo +1.96.0 test --locked --manifest-path program-template/Cargo.toml --lib --test venue_parity
```

The first command includes the pinned direct-execution and non-monotonicity
regression. The routed test is explicitly ignored until you build the router:

```sh
(cd program-template && cargo-build-sbf --tools-version v1.54 --arch v0 --manifest-path programs/titan-v3-venue-template/Cargo.toml)
cargo +1.96.0 test --locked --test ocelot_replay -- --ignored --nocapture
```

The final command tests both bundled snapshots and fails if the router binary is
missing. Optional `OCELOT_SNAPSHOT` (directory or `.bin.gz`) and `OCELOT_MARKET`
override the fixture. No deployment command is needed.

The upstream RPC suites (`tests/your_venue.rs` and the program's
`tests/your_venue_route.rs`) remain diagnostic acceptance gates. They cannot pass
while production quoting is disabled, and their LiteSVM 0.6.1 cannot load the
captured Phoenix EM_SBPF executable. Offline replay uses LiteSVM 0.16.0 while
preserving Titan's Solana 2 public types. Do not interpret an RPC-less `cargo test`
run, or the template's structural scorecard, as production acceptance.

## Capture fresh public state

Use a fresh directory and your read-only RPC URL; the example makes no write RPCs:

```sh
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com RUST_LOG=info \
  cargo +1.96.0 run --locked --example probe -- /tmp/ocelot-titan-capture \
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

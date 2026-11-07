# Titan native issuer integration

Status as of 7 November 2026: **native issuer kernel implemented; complete native
Phoenix matching provider still required before production quoting**. Source and reproducible tests are in
[the standalone adapter package](README.md).
Titan submission: draft [upstream PR #10](https://github.com/Titan-Pathfinder/integration-template/pull/10),
opened November 6 after Titan requested a fork-based PR. The public
[Ocelot fork](https://github.com/Ocelot-fi/integration-template/tree/nik-suri/titan-integration)
contains only the standalone adapter, public-chain fixtures and screened technical handoff,
initially at commit `d163ddecf760e9752f4371688d53d6ff1c8b4601`, based on upstream
`c0bb5c9b7f7fc7d00f80c24852ef36942fc8464d`. No private application history or business notes
were exported. The initial Rust source, manifests, lockfiles and fixtures matched the package at Ocelot
commit `374d612b3f61733ce658824f484f7ed229134eef`; README links and handoff context were adapted
for standalone use. Shared acceptance assertions are byte-identical to upstream.
This is integration review material, not a live Titan listing or a vault upgrade.

**Titan feedback, 7 November:** Kevin reviewed the PR and confirmed that Titan needs Rust
offchain quoting code, does not use simulations inside `fn quote`, and requires it to run
under one microsecond. This confirms the template performance constraint and rules out the
current local execution oracle as the production quote implementation. Simulation remains
useful for checking independently calculated quotes in tests. The owner subsequently
instructed us to proceed on the assumption that Phoenix lot boundaries are acceptable;
curve clarification is no longer the implementation blocker. This does not itself record
formal Titan acceptance of the derivative convention, runtime-input policy or adapter ABI.

**Implementation, 7 November:** [native quoting details](NATIVE_QUOTING.md)
and `src/ocelot/quote.rs` now contain the checked issuer math, shared lot sizing,
withdrawal/finance limits and cached native-provider interface. Generic instruction
construction uses native quotes with no VM fallback. Reference execution is feature-gated
and explicitly loaded; default library builds have no LiteSVM dependency. A missing
provider leaves the venue uninitialized and quotes rejected.

Tests compare 126 exact outputs and one matching finance-margin rejection against
captured deployed programs across both directions and lot edges. The fixed-price test
provider is bounded to these snapshots and is not a production matcher. The release
benchmark measured batch-mean p50 **75.1 ns**, p99 **107.3 ns** over one million calls
for the **issuer plus fixture provider only**; these are not per-call percentiles or
complete integration latency. Full native Phoenix execution remains the concrete missing
component, including risk-aware spline uncrossing and fees; the public SDK does not supply
that complete implementation. The package documents the provider contract and question
for Phoenix/Titan, without assuming unlimited maker capacity or publishing raw depth as
executable liquidity.

## What works

The adapter discovers Ocelot Phoenix markets, reads the finance and venue state,
builds v3 mint/redeem transactions and executes them through Titan's template
router. The router signs for its PDA and preserves existing token balances.
Local replay of the captured mainnet programs matched direct execution atom for
atom for SOL3L and SOL2S in both directions. Tests cover new and existing Titan
ATAs, reserved balances, mismatched runtime amounts, minimum output, expired
deadlines, stale finance revisions, market pause, deposit cap, and failed refresh.

All executions were local simulations with synthetic user token balances and a
synthetic warmed address lookup table. They used no wallet keys and submitted no
transactions. This proves the captured execution paths; it does not prove live
routing acceptance, all market states, arbitrary route graphs, or future upgrades.

## Recorded lot behavior and current routing assumption

Titan's template requires exact atom outputs, nondecreasing output with increasing
input, a positive nonincreasing marginal derivative, and quotes with no heap
allocation averaging under 1 microsecond. Ocelot mints and redeems by executing
Phoenix perp lots. Execution costs change discretely when the order gains a lot.
NAV minus the issuer fee does not capture this.

The committed fixtures reproduce the following counterexamples. Each comparison
uses the same captured state; all figures are raw atoms, with six token decimals:

| Market / direction | Input A | Output A | Input B | Output B |
| --- | ---: | ---: | ---: | ---: |
| SOL3L mint | 601,134 | 557,415 | 601,135 | 556,844 |
| SOL3L redeem | 566,095 | 607,315 | 566,096 | 607,201 |
| SOL2S mint | 901,900 | 879,644 | 901,901 | 879,533 |
| SOL2S redeem | 845,851 | 863,062 | 845,852 | 862,448 |

For example, one more USDC atom at the SOL3L mint boundary produces 571 fewer LT
atoms. These are actual program replays, not a hypothetical curve. A smooth NAV
approximation cannot be both exact and satisfy Titan's current acceptance rules.
The owner has asked us to proceed on the assumption Titan can handle these steps.
The native issuer kernel preserves them. `price` is the within-lot slope with integer
rounding relaxed, not a derivative at a jump or a globally concave substitute.
`TradingVenue::quote()` rejects calls until a native Phoenix provider is installed.
The feature-gated `reference_quote()` method is available for correctness verification. Its observed
debug timings were about 32–41 milliseconds, not a release performance benchmark
and not compliant with Titan's quoting budget.

## Questions to send Titan

1. **Native Phoenix execution quoter (also ask Jarry/Phoenix):** Do you have native
   Rust quoting/matching code for cached accounts covering spline uncrossing/slide
   prices, maker risk caps and exact taker fees? We have implemented Ocelot's issuer
   math and can validate against deployed-program replays. If no reusable library
   exists, can Phoenix provide reference code and test vectors for those rules?
   The complete path must stay under one microsecond with no simulation.
2. **Confirm routing convention while implementation proceeds:** We preserve the
   discrete output steps. Is the local slope between lot boundaries the `price`
   convention you want, or do you use another marginal-price convention for
   discrete venues? We are proceeding per owner guidance, rather than waiting on
   this to implement the native quoter.
3. **Can initial routes pin the quoted Ocelot input?** Our v3 order size, repeated
   deposit amounts and output guard are built for that input. The adapter rejects
   a different runtime amount. Can initial USDC ↔ LT routes use this, and how
   should Titan supply per-leg slippage/deadline policy and handle intermediate
   amounts or repeated visits to the same market?
4. **Which runtime and production adapter ABI should we target?** The supplied
   LiteSVM 0.6.1 fails to parse Phoenix's current EM_SBPF ELF. LiteSVM 0.16.0 replays
   it successfully. Our enum variant is local to the template; Titan must assign
   the production variant and confirm its current account/ALT/compute constraints.

## Reproduction evidence

All discovered mutable state was refreshed in one RPC response per market after
account discovery. Executable programdata was fetched separately and hash-pinned
below. Public RPC: `https://api.mainnet-beta.solana.com`. Capture time: approximately
01:44 UTC on 6 Nov (5 Nov in the owner's Pacific timezone).

| Fixture | State context slot | Mark slot | Finance revision |
| --- | ---: | ---: | ---: |
| SOL3L | 465928618 | 465928618 | 7 |
| SOL2S | 465928707 | 465928707 | 5 |

At input **10,000,000 atoms**, direct and routed output agree:

| Market / direction | Output atoms | Router compute units¹ | Packet bytes² |
| --- | ---: | ---: | ---: |
| SOL3L mint | 9,263,379 | 434,426 | 509 |
| SOL3L redeem | 10,726,165 | 431,846 | 505 |
| SOL2S mint | 9,752,004 | 389,242 | 509 |
| SOL2S redeem | 10,196,606 | 414,346 | 505 |

¹ Local LiteSVM 0.16.0 measurements with existing Titan ATAs; runtime features and
SPL implementations can differ from mainnet. ATA creation adds compute. These are
not live fee/latency guarantees. ² One leg with a synthetic ALT containing the
needed addresses; a usable production ALT and actual route packet still need
Titan validation.

Executable SHA-256:

- Ocelot: `0e305e8e8d1e0fafada58cb272c945b3a83ff66bc46e2ae930d9440b41a06423`, matching the recorded 2 Nov production release.
- Phoenix: `1dff93143cad98a346615480762276afe1c62ffa5bc13a0e639e6070aafdf6c5`.
- Ember: `cbf33bb02b0f6b30dcc8083e80ee7e3e9565647236b084074fc754aa6753ded0`.
- Locally built Titan template router: `4b85a025957265b03b1df16ac96f26f231032985c0d020ffffbc0c5f3ac6eaaf` (not deployed).

Fixture archive SHA-256:

- `sol3l-465928618.bin.gz`: `14cba3bf749757fb82109126b36365790a524e6bfdd7c30a1594c3aaea9ef8be`.
- `sol2s-465928707.bin.gz`: `ebc3aa4afa00a57c5014ebb8bfd2af7256f86d2000dbfec9c52230199b4e7d8b`.

## Acceptance and next steps

November 7 selected offline host tests: **21 passed**, including native issuer
coverage and both routed fixtures. Program host tests and enum parity: **3 passed**.
Default-feature native/decoder/unit coverage separately passes without the reference VM.
Host `cargo check --locked --all-targets` and targeted Clippy with `-D warnings` pass.
The SBF router builds with `cargo-build-sbf` 2.3.0 / platform tools v1.54.
The fork checkout independently repeated all 18 targeted tests with no skips, all-target
compilation, and the SBF build on November 6; the router hash matches the value above.
The upstream shared quote and router suites are preserved; **production quote
acceptance, allocation, speed and concavity gates have not passed**. The old
runtime's Phoenix ELF failure is not bypassed by weakening its assertions.

Before activation: supply the native Phoenix provider and benchmark the combined path;
confirm the routing/runtime/input contract and pass the agreed full execution/quote suite;
verify Titan's actual router, production ALT, supported route graphs and market
allowlist; complete production listing and release checks before routing real transactions. The upstream draft PR is submitted for engineering review;
production quoting and live Titan routing remain disabled.

# Titan native issuer integration

Status as of 5 November 2026: **execution adapter implemented; production quote
model unresolved**. Source and reproducible tests are in
[the standalone adapter package](README.md).
This is integration review material, not a live Titan listing or a vault upgrade.

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

## Concrete incompatibility with the template

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
`TradingVenue::quote()` therefore returns an explicit error. The distinct
`reference_quote()` method is available for correctness verification. Its observed
debug timings were about 32–41 milliseconds, not a release performance benchmark
and not compliant with Titan's quoting budget.

## Questions to send Titan

1. **Can your router support this discrete issuer curve?** At Phoenix lot
   boundaries our exact output can decrease slightly as input increases (example
   above). Does Titan support an exact-size or RFQ-style path outside the
   template's monotone/concave optimizer? If not, what quote/execution contract
   should we target before making a vault-program change?
2. **Do you have a Phoenix-perps execution-aware quoter or a supported simulation
   path we can reuse?** We can replay the deployed programs exactly, but that does
   not meet the template's allocation-free, sub-microsecond budget. NAV alone is
   insufficient; quotes must account for fills, fees, lot rounding and venue
   availability. What latency and cache/update requirements apply to your proposed
   path?
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

Selected offline host tests: 15 passed, including both routed fixtures when the
explicit replay test is enabled. Program host tests and enum parity: 3 passed.
Host `cargo check --locked --all-targets` and targeted Clippy with `-D warnings` pass.
The SBF router builds with `cargo-build-sbf` 2.3.0 / platform tools v1.54.
The upstream shared quote and router suites are preserved; **production quote
acceptance, allocation, speed and concavity gates have not passed**. The old
runtime's Phoenix ELF failure is not bypassed by weakening its assertions.

Before activation: agree the quote model and runtime/input contract with Titan;
implement and benchmark that model; pass the agreed full execution/quote suite;
verify Titan's actual router, production ALT, supported route graphs and market
allowlist; complete production listing and release checks. This package is for engineering
alignment; production quoting and live Titan routing are not enabled.

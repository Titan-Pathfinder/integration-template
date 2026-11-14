# Native midpoint policy, 14 November 2026

This implements the selected five-bp **final-output** discount for raw
locked/crossed Phoenix spline books. It is a native production-adapter path for
engineering review, not an experimental factory or a Hawkeye/VM fallback. It is
not Titan acceptance, routing activation or an on-chain program change.

## Policy and implementation

- Raw best bid below best ask, or one-sided book: use the prior native provider
  unchanged. No new output discount applies.
- Raw best bid equal to or above best ask: recenter each active nonzero maker
  spline on the common raw midpoint. Retain widths, offsets, fills, expiry and
  risk fields. For a half-tick midpoint, prepare a ceiling model for buys and a
  floor model for sells. Mint/redeem select the side of their actual hedge order,
  including short products. An integer midpoint shares one prepared provider.
- Run native preparation and its maker/taker risk, FIFO reserves, fees and
  withdrawal checks on the model. Keep original accounts in the venue, and retain
  issuer lot sizing, finance checks and advertised range handling.
- After the existing issuer output curve, quote
  `floor(curve_output * 9995 / 10000)` and multiply the marginal price by 0.9995.
  This applies exactly once, including consistent zero-input spot pricing. A
  positive input whose discounted output rounds to zero declines.
- The discount is not a fee, collateral deduction, changed order size or
  transaction minimum. Users receive actual execution output. Titan's route-level
  minimum remains the user-slippage protection; standalone instructions require
  a positive caller-supplied minimum. The agreed deadline remains `i64::MAX`.
- Refresh from one supplied account batch. Parsing/freshness/dependency errors
  are not fallback triggers. Failed refreshes clear prepared quotes; clock-only
  refreshes re-evaluate liquidity expiry. No RPC or VM is used in preparation or
  quoting, and quotes remain allocation-free.

`NativePhoenix::prepare` remains the strict non-crossed primitive. The default
`NativePhoenixFactory` selects the midpoint policy explicitly. The trait's
`is_midpoint_approximation()` flag drives the discount; ordinary/custom providers
default to false. `uses_midpoint_approximation()` exposes the current prepared
mode for diagnostics, without allowing callers to mutate it.

## Known limitation: this is not a universal underquote guarantee

The common-midpoint model is not Phoenix's slide-price calculation. The five-bp
discount is a chosen approximation allowance, not a proven maximum-error bound.
Native checks still run, but their modeled price/capacity estimates need not
equal the actual adjusted book. Do not describe this path as conservative across
all possible states or silently treat Titan's earlier native-provider sign-off
as approval of this new policy.

Two retained production-path regressions illustrate the limitation:

| Controlled state | Input | Quoted output after 5 bp | Actual output |
| --- | ---: | ---: | ---: |
| `sol3l-465928618`, maker bid raised into a four-tick crossing | 346.554022 USDC | 320.644986 SOL3L | 320.592122 SOL3L |
| `sol3l-467430913`, maker bid raised into an eight-tick crossing | 480.611111 USDC | 507.192066 SOL3L | 506.741449 SOL3L |

The four-tick case remains about 1.6489 bp too optimistic, the eight-tick case
about 8.8924 bp. Both execute with a permissive minimum and reject if required to
deliver the full buffered quote. Actual user slippage may tolerate a smaller
shortfall; the router must still enforce the final user-selected minimum.
The earlier one-tick, floor-rounded buy counterexample is covered by the new
direction-aware rounding. That improvement does not fix integer-midpoint errors.

These are controlled mutations of real account captures, not observations of
live failure frequency. No guarantee of reachability through every valid maker
update sequence is inferred. The short live sampling window had 101 normal,
18 locked and one one-tick crossed observation; it is not a long-term availability
study. No new crossing-width cutoff or adaptive buffer is introduced here.

## Integrated validation

The new tests exercise the default venue/factory, not an injected approximation.
They verify original account preservation, exact discount/price scaling, unchanged
normal output/bounds/instructions, both trade directions, no quote allocations,
repeat refreshes, normal/locked/crossed transitions, missing/malformed or stale
state rejection and clock-only expiry. Unknown makers do not gain liquidity by
passing through midpoint preparation.

- 67 reference-feature tests pass under `release-debug` with allocation guards.
  Expensive research/live/benchmark cases are explicitly excluded from this
  selection and reported separately.
- 42 default-feature tests pass; four benchmarks remain ignored.
- Three program host/enum parity tests pass. Program sources and SBF artifacts
  are unchanged; no rebuild or deployed upgrade is claimed.
- Routed slippage/deadline tests now include both tokens on both real locked and
  crossed captures, alongside the four earlier normal captures. The local test
  guard verifies 144 successful routes and 96 atomic rollbacks after successful
  Ocelot/Phoenix execution. It is not Titan's production guard. Timestamp-only
  tests record 32 direct/routed successes at +21 seconds and 16 explicit expired
  controls; holding slots fixed is not evidence of quote lifetime after signing.

### Original captured books and shared checks

The dense production-provider sweep compares **188,680** actual issuer executions
on the unmodified locked/crossed SOL captures, across both markets and both trade
directions. Every lot edge (before/at/after), interiors, range endpoints and
additional fixed/random amounts pass: zero overquotes, execution failures or
range holes. Maximum observed underquote is 15.8219 bp, including the five-bp
policy discount; this is a fixture result, not a universal bound.

All **40 unchanged shared assertions** pass across eight fixture/market cases:
construction/allocation, zero-input price, output monotonicity, price monotonicity
and mean-value theorem. A read-only loopback fixture RPC supplies the captures.
Shared old-VM execution and speed cases are excluded; compatible actual-program
replays and the complete native benchmark below supply separate evidence. No
skipped test is counted as a pass. Both Clippy configurations and formatting pass.

### Controlled production-provider sweep

Four historical fixtures; one maker moved to lock and then cross in each
direction. Every lot boundary's preceding/at/following atom and an interior point
are checked for curve shape and holes; executions sample every 64th lot, range
ends, fixed ticket sizes and deterministic random inputs.

| Crossing width | Execution comparisons | Quotes above actual output | Worst overquote |
| --- | ---: | ---: | ---: |
| Locked | 7,448 | 0 | 0 |
| 1 tick | 7,448 | 0 | 0 |
| 2 ticks | 7,448 | 0 | 0 |
| 4 ticks | 7,448 | 588 | 1.9192 bp |
| 8 ticks | 7,448 | 3,456 | 8.8924 bp |
| 16 ticks | 7,448 | 3,691 | 22.8974 bp |
| 32 ticks | 7,448 | 3,709 | 51.1436 bp |

The sweep checks 2,704,632 quote inputs and compares 52,136 executions; 11,444
quotes exceed actual output in the wider crossings. Its largest underquote is
101.0880 bp in controlled stress states.

These counts repeat many amounts in constructed states; they are not a live
failure probability. All permissive-floor executions succeed in this sweep.
Passing sampled narrow crossings does not establish a safe width threshold.
Curve checks use the shared two-output-atom rounding tolerance.

## Performance

Release benchmark on the local Mac, 100 batches of 10,000 complete venue quotes
and 100 full cached `update_state` calls per case. Quote percentiles describe
**batch means**, not individual-call tail latency. No RPC/VM time is hidden in
the measured calls; reference execution is not used by the production provider.

| Snapshot | Quote median / p99 | Refresh median / p99 |
| --- | ---: | ---: |
| Normal SOL3L | 0.112 / 0.195 µs | 1.336 / 1.555 ms |
| Locked SOL3L | 0.123 / 0.154 µs | 1.458 / 1.906 ms |
| Crossed SOL3L | 0.121 / 0.172 µs | 1.887 / 3.666 ms |
| Crossed SOL2S | 0.121 / 0.145 µs | 1.769 / 2.156 ms |

These are local measurements, not Titan production-host guarantees. The odd-tick
crossed book prepares separate buy/sell models; the integer midpoint shares one.
[Machine-readable measurements](MIDPOINT_MEASUREMENTS.json) retain per-case
results, commands and unchanged router/guard artifact hashes.

## Reproduction

Run from the standalone package root (or `packages/titan-integration`):

```sh
cargo +1.96.0 test --profile release-debug --locked --features reference-simulation --test ocelot_midpoint
# Dense default-provider replays on the original locked/crossed captures.
cargo +1.96.0 test --release --locked --features reference-simulation --test ocelot_midpoint production_execution_sweep -- --ignored --nocapture
# Explicit stress study: reports known overquotes, not an assertion of universal safety.
MIDPOINT_SWEEP=controlled cargo +1.96.0 test --release --locked --features reference-simulation --test ocelot_midpoint production_execution_sweep -- --ignored --nocapture
cargo +1.96.0 test --release --locked --test ocelot_midpoint benchmark_midpoint_quote_and_refresh -- --ignored --nocapture
```

No network, wallet signing or chain submission occurs in these replays.
Unsupported oracle/Ember configurations and ANT activation remain outside scope.
Same-snapshot results do not guarantee execution after wallet/landing delay.
Titan must review the approximation/error tradeoff and validate its own final
route minimum before activation. Historical Hawkeye experiments were archived
separately and are not included in the runtime or standalone submission.

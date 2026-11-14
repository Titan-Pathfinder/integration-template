# Standalone midpoint export validation, 14 November 2026

Exported source: `25cdb9d81afa0e960bbfcd6b1471838836ab4ead`. The default adapter now uses direction-aware native
midpoint preparation with a five-bp final-output discount only for locked/crossed
books. Normal quotes and the on-chain contract are unchanged. This is an explicit
approximation with known overquotes, not a universal execution-output bound.

## Current standalone results

| Check | Result |
| --- | --- |
| Reference-feature `release-debug` suite | 67 passed, allocation guards enabled |
| Default-feature `release-debug` suite | 42 passed; 4 benchmarks explicitly ignored |
| Unchanged shared assertions, eight fixture/market cases | 40 passed |
| Program host/enum parity tests | 3 passed |
| Reference/default Clippy with warnings denied | Both pass |
| Formatting | Pass |
| Rust/tests/manifests/locks/measurements/fixtures | Byte-identical to committed source |
| Shared quote/router assertion files | Byte-identical to pinned Titan upstream |
| Router/test-guard artifacts | Identical to prior independently rebuilt artifacts |
| Standalone source/privacy screen | Pass |

The reference selection excludes named benchmark/live research cases and the
expensive dense execution sweep. Its five midpoint tests exercise the default
provider, unchanged normal output/instructions, state preservation, refresh mode
transitions, expiry, original-state executions and retained wider-crossing error
regressions. It includes 144 routed successes and 96 atomic rollbacks via a
**test-only** final guard, plus 32 timestamp-only +21-second successes and 16
expired controls. This does not test Titan's actual production guard.

The 40 shared checks cover construction/allocation, zero-input price, monotone
output/price and mean-value theorem. The old-VM execution and shared speed cases
are explicitly excluded; compatible actual-program replays and the native
benchmark supply separate evidence. Nothing skipped is counted as passed.

The dense 188,680 captured executions and controlled 52,136-execution / 2,704,632
quote study were run in the source workspace, not rerun after a byte-identical
export. Original captured books have no overquotes/failures; the controlled
study retains 11,444 overquotes in wider crossings. These are not a production
failure probability or a universal five-bp safety bound. Details and commands:
[MIDPOINT_VALIDATION.md](MIDPOINT_VALIDATION.md),
[MIDPOINT_MEASUREMENTS.json](MIDPOINT_MEASUREMENTS.json).

Cargo dependency/build caches were reused; the exported package was recompiled
from this checkout, as recorded in build logs. This is not a clean-machine
installation. Program source is unchanged, so no new SBF rebuild is claimed.
The existing separately rebuilt router/guard match the source-workspace files:

- Router: `b8997ff5ef34d8fa27a1e71fdfa1b134a6938788de920fa0e1d19b1f7d8652ba`
- Test-only guard: `9f5383d9036a493f3132dfdb4090ebd479f5237f7c5d198d6fd9215436bb090c`

## Current standalone performance

100 batches of 10,000 complete quotes and 100 complete cached refreshes per case.
Quote percentiles describe batch means, not individual-call tails. Local Mac
measurements are not production latency guarantees; normal-book timing includes
an outlier batch above 1 µs while all p99 batch means remain below 1 µs.

| Snapshot | Quote median / p99 / max (µs) | Refresh median / p99 (ms) |
| --- | ---: | ---: |
| Normal SOL3L | 0.110 / 0.415 / 1.264 | 1.302 / 2.095 |
| Locked SOL3L | 0.120 / 0.131 / 0.133 | 1.388 / 1.521 |
| Crossed SOL3L | 0.120 / 0.127 / 0.129 | 1.827 / 2.031 |
| Crossed SOL2S | 0.122 / 0.143 / 0.145 | 1.779 / 2.035 |

## Commands run

Run from the standalone package root; the shared tests were served by
`tests/support/fixture_rpc.py` using the eight cases documented in
[MIDPOINT_VALIDATION.md](MIDPOINT_VALIDATION.md).

```sh
cargo +1.96.0 test --profile release-debug --locked --features reference-simulation --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_state --test ocelot_replay --test ocelot_spline --test ocelot_slippage --test ocelot_curve --test ocelot_midpoint --test your_venue_creation -- --include-ignored --skip benchmark_ --skip fresh_spline_comparison --skip production_execution_sweep
cargo +1.96.0 test --profile release-debug --locked --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_state --test ocelot_spline --test ocelot_slippage --test ocelot_curve --test ocelot_midpoint --test your_venue_creation
cargo +1.96.0 clippy --locked --features reference-simulation --lib --test ocelot_midpoint --test ocelot_curve --test ocelot_slippage --example capture_locked -- -D warnings
cargo +1.96.0 clippy --locked --lib --test ocelot_midpoint -- -D warnings
cargo +1.96.0 fmt --check
cargo +1.96.0 test --locked --manifest-path program-template/Cargo.toml -p titan-v3-venue-template --lib --test venue_parity
cargo +1.96.0 test --profile release-debug --locked --test your_venue --no-run --message-format=json
cargo +1.96.0 test --release --locked --test ocelot_midpoint benchmark_midpoint_quote_and_refresh -- --ignored --nocapture
# Run the built shared test binary against each read-only fixture RPC:
SOLANA_RPC_URL=http://127.0.0.1:PORT OCELOT_TEST_MARKET=MARKET your_venue_test_binary --skip bound_simulation --skip random_samples --skip quoting_speed --nocapture
```

Titan review must accept the approximation/error policy and verify production
Clock delivery, range discovery, final user minimum and runtime/ABI integration
before activation. No routing activation, program upgrade or ANT validation is
implied by this export. The prior evidence below is retained as history.

## Historical standalone curve export validation, 12 November 2026

The checks below passed from the standalone checkout exported from source
`b597dddf7c5ea8bd053e7cf8507dff8ee478e84e`. Runtime and test sources, Cargo manifests/locks and all four
public-chain captures match that source byte-for-byte; documentation links and
submission status are adapted for standalone use. No application history,
business notes or credentials are included.

## Current verification

| Check | Result |
| --- | --- |
| Selected reference-feature suites | 62 passed; benchmarks/fresh-RPC diagnostics excluded |
| Selected default-feature suites | 38 passed; 3 benchmarks/diagnostics ignored |
| Program host and enum parity | 3 passed |
| Unchanged shared assertions via fixture RPC | 20 passed across four captures; no live-RPC claim |
| Curve lot-edge/interior comparisons | 273,390 accepted; no monotonicity, price or MVT violations |
| Synthetic accounting comparisons | 9,666 accepted |
| Stressed finance-range comparisons | 2,040,435 accepted; 24/80 stressed directions conservatively unavailable |
| Expanded captured-execution sweep | 578 accepted, zero overquotes or failed executions; maximum 11.699366 bp underquote |
| Stressed range execution | 616 accepted, zero overquotes or failed executions |
| Original captured execution sweep | 205 accepted and 39 matching declines; maximum underquote now 11.804777 bp |
| Existing routed parity executions | 51 pass |
| Final user-minimum model | 72 successes, 48 complete rollbacks |
| Deadline isolation | 16 successes at +21 seconds, 8 expired-deadline controls |
| Clippy, default/reference selections | Both pass with warnings denied |
| Formatting / whitespace / export source parity | Pass |
| Shared quote/router assertion sources | Byte-identical to pinned upstream |
| Router and test-guard artifacts | Match prior independently rebuilt artifacts; program sources unchanged |

Core curve math received internal review. The later mint-range and per-direction
failure-isolation fixes are included for Titan engineering re-review. The accepted
execution sweeps are same-state evidence, not guarantees after wallet delay. The
reviewer's additional mutated states reached 23.7 bp underquote; neither that nor
12 bp is a universal bound. Locked/crossed books still fail closed.

The 20 shared checks cover construction/allocation, zero-input spot price,
monotone output, monotone price and MVT, served by the read-only fixture RPC.
Shared bound/random execution simulations were filtered because the old VM
cannot load current Phoenix executables; the shared speed test was filtered in
favor of the complete native benchmark below. No skipped test is counted as a
pass. The compatible execution reference and test-only final guard do not
validate Titan's production router or private harness.

Cargo dependency/build caches were reused, with the exported package recompiled
from this checkout; the build log records its standalone source path. This is
not a clean-machine dependency installation. No program source changed, so no
new SBF rebuild was needed; the existing independently rebuilt router and guard
were verified against source-workspace artifacts:

- Router: `b8997ff5ef34d8fa27a1e71fdfa1b134a6938788de920fa0e1d19b1f7d8652ba`
- Test-only guard: `9f5383d9036a493f3132dfdb4090ebd479f5237f7c5d198d6fd9215436bb090c`

## Current standalone benchmark

```text
complete OcelotVenue::quote: 1,000,000 calls, batch-mean p50=115.7ns p99=120.8ns max=121.3ns (nearest-rank; refresh excluded)
native preparation: 100 calls p50=331.3us p99=407.8us max=462.1us (nearest-rank; cached-state decoding, excludes cache fetch/copy)
complete cached update_state: 100 calls p50=1276.2us p99=1531.3us max=1546.1us (nearest-rank; in-memory Snapshot cache, no network)
cached refresh fan-out: 4 fixture instances, 100 cycles p50=5.016ms p99=5.438ms max=6.165ms (nearest-rank; sequential; excludes network)
cached refresh fan-out: 26 fixture instances, 100 cycles p50=32.651ms p99=38.599ms max=54.117ms (nearest-rank; sequential; excludes network)
individual complete quotes: 100,000 calls p50=125ns p99=167ns p99.9=250ns max=14583ns; timer-only p50=0ns p99=42ns (nearest-rank; includes timer overhead and scheduling)
```

These measurements use a release build, two alternating supported quote inputs
and an in-memory account cache without network delivery. Batch means are not
individual-call worst-case latency. Individual timings include timer overhead
and scheduling. Fan-out repeats four fixture states rather than 26 distinct
live markets. Range discovery and preparation are outside quote timing; no
production latency guarantee is implied.

## Reproduce and review

Use [CURVE_VALIDATION.md](CURVE_VALIDATION.md#reproduce) for current suite and
fixture-RPC commands, [README.md](README.md) for router/guard builds, and
[HANDOFF.md](HANDOFF.md) for the re-review scope and acceptance questions. Run
`shasum -a 256 -c SHA256SUMS` to verify the export manifest. The historical
checkpoints below retain their original results and are superseded by this
section for the current curve candidate.

## Historical export validation, 10 November 2026

The selected checks below passed from this standalone checkout. The Rust source,
Cargo manifests/lockfiles and all four fixture archives match the source commit
in SOURCE.json byte-for-byte. The latest adapter source is
`52e54dfc0b4ed883722fee62f5fa234586194a53`. The prior quote/execution review cleared
`a8c5f9fd`; the later adapter-policy patches is not covered by that sign-off.
Documentation links/status are normalized for the public export, and HANDOFF.md
contains the screened technical handoff. No application source/history, business
notes or credentials are exported.

### Checks

| Check | Result |
| --- | --- |
| Selected reference-enabled host suite | 49 passed |
| Selected default-feature suite | 27 passed; 3 benchmarks ignored |
| Program host and enum parity | 3 passed |
| Native quote vs captured execution | 205 accepted executions; 39 matching declines; no overquotes or false accepts |
| Routed execution | 51 existing parity executions across 17 cases and 3 reserve modes |
| Final minimum/rollback model | 72 successes and 48 complete rollbacks after Ocelot/Phoenix execution |
| Standalone minimum | Positive caller floor required; zero rejected by standalone helper |
| Prior-source regressions | Zero-floor and maximum-deadline assertions fail on their preceding sources |
| Deadline isolation | 16 direct/routed successes at +21 seconds; 8 expired-deadline rejections |
| Zero-weight regression | 80 accepted executions; 48 matching rejections; 16 conservative declines |
| Clippy, default and reference-enabled selections | Both passed with warnings denied |
| Rust formatting / diff whitespace | Passed |
| SBF router and test-guard identity | Prior independent builds match; program sources and hashes unchanged by deadline patch |
| Exported source and fixture parity | Verified |
| Upstream shared quote/router assertions | Byte-identical to the pinned template |

The test-only final guard is appended after the route in one transaction. It
checks the output delta against a harness-supplied initial balance. It is not
Titan production code and must never be deployed. Failed transactions restore
all writable route accounts, allowing only the payer fee deduction. See
[SLIPPAGE_VALIDATION.md](SLIPPAGE_VALIDATION.md) for coverage and the production
acceptance request. The actual Titan production guard has not been exercised.

The deadline regression changes only Unix time, holding slot-dependent state
fixed: all four captures and both directions execute directly and through the
router after 21 seconds. An expired deadline still rejects eight controls with
`DeadlineExpired`. The historical expiry regression explicitly retains its old
bounds. This isolates the cutoff; it does not guarantee liquidity survival or
blockhash validity over a real delay.

Host checks use Rust 1.96.0 with locked dependencies. Cargo dependency caches were
reused; the integration package's prior host artifacts were cleaned before
recompiling the exported sources. This is standalone-source verification, not a
clean-machine install test.
The SBF router was independently rebuilt for the preceding slippage handoff
with cargo-build-sbf 2.3.0, platform tools v1.54 and `--arch v0`. Its source and
artifact are unchanged by this host-only deadline patch. SHA-256:
`b8997ff5ef34d8fa27a1e71fdfa1b134a6938788de920fa0e1d19b1f7d8652ba`.

The unchanged, previously independently rebuilt test-only guard has SHA-256
`9f5383d9036a493f3132dfdb4090ebd479f5237f7c5d198d6fd9215436bb090c`.

The shared RPC-dependent acceptance suites were not run. Benchmarks and optional
live-capture tests were excluded from the selected correctness suite; the prior
quote benchmark below is retained because this patch does not change quote math. No funds, signatures,
deployment, live routing or acceptance-policy changes were involved.

### Earlier November 10 export benchmark (unchanged quote implementation)

Recorded before the slippage patch on the local development host; not rerun for this instruction-only policy change:

```text
complete OcelotVenue::quote: 1,000,000 calls, batch-mean p50=94.9ns p99=104.6ns max=105.5ns (nearest-rank; refresh excluded)
native preparation: 100 calls p50=354.6us p99=482.3us max=516.3us (nearest-rank; cached-state decoding, excludes cache fetch/copy)
complete cached update_state: 100 calls p50=1271.0us p99=1706.0us max=1765.5us (nearest-rank; in-memory Snapshot cache, no network)
cached refresh fan-out: 4 fixture instances, 100 cycles p50=5.201ms p99=6.458ms max=6.576ms (nearest-rank; sequential; excludes network)
cached refresh fan-out: 26 fixture instances, 100 cycles p50=33.794ms p99=57.105ms max=89.085ms (nearest-rank; sequential; excludes network)
individual complete quotes: 100,000 calls p50=125ns p99=167ns p99.9=250ns max=26417ns; timer-only p50=0ns p99=42ns (nearest-rank; includes timer overhead and scheduling)
```

Quotes use two alternating supported fixture inputs. Batch means exclude refresh;
individual-call timings include timer overhead and scheduling. The timer-only p99
is 42 ns. In-memory cache refresh excludes network delivery; fan-out repeats the
four captured market instances rather than measuring 26 different live markets.
These measurements are not production latency guarantees. Quote and refresh
latency are separate budgets for Titan to confirm.

### Reproduce

Run from the standalone package root. Build the router before routed replay.

```sh
(cd program-template && cargo-build-sbf --tools-version v1.54 --arch v0 --manifest-path programs/titan-v3-venue-template/Cargo.toml)
(cd program-template && cargo-build-sbf --tools-version v1.54 --arch v0 --manifest-path test-programs/output-guard/Cargo.toml)
cargo +1.96.0 test --locked --features reference-simulation --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_state --test ocelot_replay --test ocelot_spline --test ocelot_slippage --test your_venue_creation -- --include-ignored --skip benchmark_ --skip fresh_spline_comparison
cargo +1.96.0 test --locked --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_spline --test ocelot_slippage --test your_venue_creation
cargo +1.96.0 test --locked --manifest-path program-template/Cargo.toml --lib --test venue_parity
cargo +1.96.0 clippy --locked --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_spline --test ocelot_slippage -- -D warnings
cargo +1.96.0 clippy --locked --features reference-simulation --lib --test ocelot_state --test ocelot_phoenix --test ocelot_replay --test ocelot_native --test ocelot_spline --test ocelot_slippage -- -D warnings
cargo +1.96.0 fmt --check
shasum -a 256 -c SHA256SUMS
```

See HANDOFF.md for the remaining host/routing policy decisions and NATIVE_QUOTING.md
for conservative refusals, pricing error, delayed execution and ANT scope.

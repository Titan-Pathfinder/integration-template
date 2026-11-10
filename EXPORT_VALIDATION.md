# Standalone export validation, 10 November 2026

The selected checks below passed from this standalone checkout. The Rust source,
Cargo manifests/lockfiles and all four fixture archives match the source commit
in SOURCE.json byte-for-byte. The final reviewed runtime is `a8c5f9fd`; source
`50d307269e9ac2260af47ca3d92371293075db0b` adds only the review-status documentation.
Documentation links/status are normalized for the public export, and HANDOFF.md
contains the screened technical handoff. No application source/history, business
notes or credentials are exported.

## Checks

| Check | Result |
| --- | --- |
| Selected reference-enabled host suite | 46 passed |
| Selected default-feature suite | 26 passed; 3 benchmarks ignored |
| Program host and enum parity | 3 passed |
| Native quote vs captured execution | 205 accepted executions; 39 matching declines; no overquotes or false accepts |
| Routed execution | 51 executions across 17 cases and 3 reserve modes |
| Zero-weight regression | 80 accepted executions; 48 matching rejections; 16 conservative declines |
| Clippy, default and reference-enabled selections | Both passed with warnings denied |
| Rust formatting / diff whitespace | Passed |
| Independent SBF router rebuild | Passed; identical SHA-256 below |
| Exported source and fixture parity | Verified |
| Upstream shared quote/router assertions | Byte-identical to the pinned template |

Host checks use Rust 1.96.0 with locked dependencies. Cargo dependency caches were
reused; this is a standalone-source verification, not a clean-machine install test.
The SBF router was rebuilt in this checkout with cargo-build-sbf 2.3.0, platform
tools v1.54 and `--arch v0`. SHA-256:
`4b85a025957265b03b1df16ac96f26f231032985c0d020ffffbc0c5f3ac6eaaf`.

The shared RPC-dependent acceptance suites were not run. Benchmarks and optional
live-capture tests were excluded from the selected correctness suite; the complete
native quote benchmark was then run separately in release mode. No funds, signatures,
deployment, live routing or acceptance-policy changes were involved.

## Fresh export benchmark

Recorded on the local development host, after correctness/build work completed:

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

## Reproduce

Run from the standalone package root. Build the router before routed replay.

```sh
(cd program-template && cargo-build-sbf --tools-version v1.54 --arch v0 --manifest-path programs/titan-v3-venue-template/Cargo.toml)
cargo +1.96.0 test --locked --features reference-simulation --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_state --test ocelot_replay --test ocelot_spline --test your_venue_creation -- --include-ignored --skip benchmark_ --skip fresh_spline_comparison
cargo +1.96.0 test --locked --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_spline --test your_venue_creation
cargo +1.96.0 test --locked --manifest-path program-template/Cargo.toml --lib --test venue_parity
cargo +1.96.0 clippy --locked --lib --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_spline -- -D warnings
cargo +1.96.0 clippy --locked --features reference-simulation --lib --test ocelot_state --test ocelot_phoenix --test ocelot_replay --test ocelot_native --test ocelot_spline -- -D warnings
cargo +1.96.0 fmt --check
cargo +1.96.0 test --release --locked --test ocelot_phoenix benchmark_complete_native_quote -- --ignored --nocapture
shasum -a 256 -c SHA256SUMS
```

See HANDOFF.md for the remaining host/routing policy decisions and NATIVE_QUOTING.md
for conservative refusals, pricing error, delayed execution and ANT scope.

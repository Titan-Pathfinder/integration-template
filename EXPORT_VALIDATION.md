# Standalone export validation, 10 November 2026

The selected checks below passed from this standalone checkout. The Rust source,
Cargo manifests/lockfiles and all four fixture archives match the source commit
in SOURCE.json byte-for-byte. The latest adapter source is
`52e54dfc0b4ed883722fee62f5fa234586194a53`. The prior quote/execution review cleared
`a8c5f9fd`; the later adapter-policy patches is not covered by that sign-off.
Documentation links/status are normalized for the public export, and HANDOFF.md
contains the screened technical handoff. No application source/history, business
notes or credentials are exported.

## Checks

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

## Earlier November 10 export benchmark (unchanged quote implementation)

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

## Reproduce

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

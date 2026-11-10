# Ocelot integration handoff, 10 November 2026

The bundled conservative native Phoenix provider is installed by default.
Internal review closed the reported same-state quote/execution blockers. This
submission is for Titan engineering review, not activation or an agreed error bound.
See [implementation and evidence](NATIVE_QUOTING.md), [build/replay commands](README.md),
and [source/export provenance](SOURCE.json).

## What is included

- Market discovery and coherent final-batch account decoding, including absent
  dependency subscriptions and cold/warm zero-supply initialization.
- Native allocation-free issuer/spline quotes with maker/taker risk, fee overrides,
  conservative FIFO-uncross reserves, oracle eligibility and withdrawal bounds.
- Exact-input Ocelot v3 mint/redeem instructions and the Titan template CPI adapter.
- Four captured SOL3L/SOL2S states, executable reference replay and regression tests.

## Measured evidence

- 46 selected host tests, 26 default-feature tests, 3 program/parity tests and both
  Clippy configurations pass in this exported checkout. Details and commands are recorded in
  [EXPORT_VALIDATION.md](EXPORT_VALIDATION.md).
- 205 accepted captured-state quotes execute without overquoting. All 39 baseline
  declines are also rejected by reference execution. 200/205 comparisons are
  within 1 bp; the maximum underquote is 4.144072 bp, on one full SOL3L exit.
- 51 routed executions cover four captures, both directions, larger trades/full
  exits and three reserved-balance modes. Direct/routed output must match exactly.
- Zero-weight spot/perp validity regression: 80 accepted executions, 48 matching
  execution rejections and 16 intentional conservative declines. A zero-weight
  book does not require freshness under the pinned policy.
- Recorded complete-quote batch-mean p50/p99/max: 96.6/479.1/543.6 ns. Individual
  call p50/p99/p99.9: 125/167/250 ns, maximum 39,000 ns including timer and OS
  scheduling. Complete cached refresh p50/p99/max: 1,270.7/1,486.3/1,511.3 us.
  These are local measurements, not production guarantees; fresh exported-checkout
  timings and limitations are recorded with its validation.

The public SDK layout pin is Rise v0.9.3 commit
`2ce47221ef04a8d5879d39d31e82f9e92021753e`. Unknown oracle policy and Ember
configuration fail closed. Conservative refusals include crossed splines, hot
issuer children, unsupported maker positions and special-priced markets. ANT
oracle policy/special pricing are outside validated scope and require separate
configuration and execution validation before activation.

## Decisions needed from Titan

1. Can the final account batch be coherent, with clock-only changes triggering a
   refresh? Discovery data is discarded, but the adapter cannot detect a torn
   final batch or refresh without host callbacks.
2. What quote lifetime and output-floor/deadline policy should be used? The
   reference defaults are 1% and 20 seconds. Delayed original transactions can
   settle below their quote or reject; the output floor is not an error target.
3. What final-output error and quote/refresh timing budgets are acceptable?
   Confirm the 4.144072 bp worst captured underquote and conservative availability
   restrictions against Titan's live-state corpus; these samples are not bounds.
4. Does declining a weighted book older than its cached threshold (200 slots in
   these captures) matter during quiet markets? Phoenix may execute these states.
5. Can initial routes preserve the exact quoted issuer input and avoid independently
   quoted repeated visits to the same market? Variable intermediate amounts are
   rejected. Confirm the slope convention for discontinuities at Phoenix lot steps.
6. Confirm production venue enum/ABI, runtime, compute budget, ALTs, route graphs
   and market allowlist. LiteSVM 0.16.0 replays Phoenix's current EM_SBPF artifact;
   the template's 0.6.1 runtime cannot. Simulation is verification-only, feature-gated.

Upstream shared quote/router assertions remain unchanged. RPC-dependent template
acceptance suites have not passed and are not replaced by these selected offline
tests. No on-chain program upgrade, deployment or live routing is included.

## Artifact identity

The independently built template router must match SHA-256
`4b85a025957265b03b1df16ac96f26f231032985c0d020ffffbc0c5f3ac6eaaf`.
Fixtures contain public captured accounts and executable programdata. Synthetic
user balances and an ALT support offline execution without mainnet funds.
Archive hashes and reproduction details are in tests/fixtures/README.md and
NATIVE_QUOTING.md; SHA256SUMS covers the exported files.

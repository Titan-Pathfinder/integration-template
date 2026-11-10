# Ocelot integration handoff, 10 November 2026

The bundled conservative native Phoenix provider is installed by default.
Internal review closed the prior same-state quote/execution blockers at `a8c5f9fd`.
The later router-slippage patch follows that sign-off and is locally verified;
it still needs Titan's production-router acceptance. This
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

- 49 selected host tests, 27 default-feature tests, 3 program/parity tests and both
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

## Slippage update and concrete production check

Per Kevin's November 10 guidance, the generic builder now encodes zero Ocelot
minimum and the CPI adapter accepts it. `build_with_minimum` remains the positive
minimum helper for standalone use. Quote math is unchanged. Titan subsequently
agreed to removing the 20-second adapter cutoff; the required deadline is now
`i64::MAX`. The 100 bps IOC band, finance revision and exact-input restrictions
remain in force. Normal transaction expiry belongs to Titan/frontend/wallet and Solana.

The sample router has no final user minimum. A separate **test-only** SBF guard
models that check in the same local transaction: 72 successes and 48 complete
rollbacks after successful Ocelot/Phoenix execution. The test inspects all writable
route accounts and permits only the payer fee to differ on failure. It does not
test or replace Titan's production router. Never deploy the test guard.

**Please run the updated adapter against your production router/private harness:**
verify mint and redeem with 0.5% user slippage, including a below-minimum result
after Ocelot succeeds, equality/success controls, and existing user output balances.
Confirm the complete rollback and correct final minimum after fees/rounding. Also
test a tolerance above 1% so Ocelot's removed fixed floor cannot override the user.
See [SLIPPAGE_VALIDATION.md](SLIPPAGE_VALIDATION.md) for the exact acceptance cases.

A timestamp-isolation regression adds 16 direct/routed successes after 21 seconds
and eight explicit expired-deadline rejections. The slot is held fixed to isolate
the removed cutoff; this does not promise liquidity survival or blockhash validity
after a real signing delay. The program deadline check itself is unchanged.

## Confirmed host contract and remaining decisions

- Titan confirms a coherent single-slot account view and `update_state` as the
  only venue-update path. Clock is already subscribed; verify actual Clock-only
  delivery removes expired liquidity on your host.
- Titan confirms frontend/API user slippage is enforced in its production router.
  The adapter is now aligned; the production execution cases above remain open.
- Titan agreed to removing the adapter deadline; this policy is implemented.
  Wallet signing delay remains unbounded, and automatic re-simulation/rebuilding
  was not promised. The max-timestamp field adds no practical expiry cutoff.
- Accept quote/output-error and refresh budgets against your corpus, including
  the recorded 4.144072 bp worst captured underquote. Local samples are not bounds.
- Review quiet-market availability: weighted books older than their cached
  threshold decline even when Phoenix may execute under the pinned rules.
- Confirm exact quoted issuer input, intermediate/repeated-market restrictions,
  slope convention at lot steps, production venue enum/ABI, runtime, compute, ALTs
  and market allowlist. ANT oracle policy/special pricing remains unsupported.

Upstream shared quote/router assertions remain unchanged. RPC-dependent template
acceptance suites have not passed and are not replaced by these selected offline
tests. No on-chain program upgrade, deployment or live routing is included.

## Artifact identity

The independently built template router must match SHA-256
`b8997ff5ef34d8fa27a1e71fdfa1b134a6938788de920fa0e1d19b1f7d8652ba`.
Fixtures contain public captured accounts and executable programdata. Synthetic
user balances and an ALT support offline execution without mainnet funds.
Archive hashes and reproduction details are in tests/fixtures/README.md and
NATIVE_QUOTING.md; SHA256SUMS covers the exported files.

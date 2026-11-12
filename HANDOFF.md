# Ocelot integration handoff, 12 November 2026

This update addresses the marginal-price and lot-edge curve findings from Titan's
review. It includes the subsequent mint-range and direction-isolation fixes for
engineering re-review. Locked/crossed-book rejection is unchanged and remains an
open availability issue. This submission does not activate routing.

## Implementation

- Account refresh prepares capacity-limited cumulative Phoenix spline depth,
  maker/taker risk, effective fees and withdrawal limits from a coherent account
  batch, with Clock and absent dependencies subscribed.
- Refresh also prepares a conservative concave output envelope for mint and
  redeem. Quote returns its output and matching marginal rate, while retaining
  the original whole-lot execution checks and rejecting an estimate above the
  native output bound. No RPC, VM execution or allocation occurs in quote.
- The venue's `bounds()` override restricts mint and partial-redemption ranges
  to avoid finance failures between otherwise valid lot sizes. Individually
  valid amounts outside those ranges can still quote directly. Titan must honor
  the override instead of independently binary-searching the raw quote domain.
- A curve representability error disables only its own direction. Shared account,
  oracle, risk and locked/crossed geometry failures still invalidate the refresh.
- The exact-input v3 instruction/CPI contract is unchanged by this curve update:
  generic routed instructions carry zero venue minimum, standalone instructions
  require a positive minimum, and the required deadline is `i64::MAX` as agreed.

See [curve derivation and regressions](CURVE_VALIDATION.md),
[provider restrictions](NATIVE_QUOTING.md), [build instructions](README.md),
[standalone verification](EXPORT_VALIDATION.md) and [source provenance](SOURCE.json).

## Evidence and scope

The source-workspace verification includes 62 reference-feature tests, 38 default
tests and 20 unchanged shared quote/construction checks through a read-only local
fixture RPC. It includes 273,390 lot-edge/interior comparisons, 9,666 synthetic
accounting comparisons, 2,040,435 stressed range comparisons and accepted execution
sweeps of 578 and 616 cases without overquotes or failed accepted executions.
Standalone repeat results are recorded in EXPORT_VALIDATION.md.

The original 205 accepted execution comparisons and 39 matching declines remain.
The maximum underquote is now 11.804777 bp in those captured states, predominantly
a tiny-trade effect; the additional reviewer stress cases reached 23.7 bp. Neither
number is a universal error ceiling. Quote/error acceptance remains Titan's decision.

Shared assertion source is unchanged. The 20 shared checks cover construction,
zero input, monotone output, monotone price and mean-value theorem across four SOL
snapshots. Shared VM simulations and live-RPC acceptance are not claimed as passed;
execution comparisons use the compatible captured-program reference runtime.

Core curve review found no output-bound or curve-shape defects. It identified
the mint-range and coupled-direction issues addressed in this update; those
follow-up fixes still need independent engineering review. The review example's
limit is 20,200 bps, as used by its probe, rather than the 22,000-bps prose label.

## Requested Titan re-review

1. Review the conservative curve, its marginal price and both finance-range
   bounds against your private harness, including lot edges and stressed states.
2. Confirm production range discovery calls our `venue.bounds()` override.
3. Confirm acceptable output error by ticket size and your quote/refresh budgets.
4. Verify Clock-only delivery invokes our subscribed `update_state` so prepared
   Phoenix liquidity expires even without another account change.
5. Verify the final user minimum through the production router: both mint and
   redeem with 0.5% slippage, below-minimum rollback after Ocelot succeeds, equality
   controls, existing output balances and a tolerance above 1%.

The template router does not contain Titan's final user guard. A separate local
test-only guard verifies 72 successes and 48 atomic rollbacks; it must never be
deployed or used as a substitute for the production guard. See
[slippage acceptance cases](SLIPPAGE_VALIDATION.md). The timestamp-only regression
retains 16 direct/routed successes at +21 seconds and 8 explicit expired-deadline
rejections; holding slots fixed does not prove liquidity survives signing delay.

## Still open

Locked/crossed spline handling, common-midpoint/slide-price support and future
maker-update timestamps remain separate work. Unknown oracle/Ember configurations
fail closed. ANT3L/ANT2S remain outside the validated profile. Quiet-market book
staleness can reduce availability. Exact-input route restrictions, production
ABI/runtime, user-minimum/Clock checks and error/timing acceptance remain activation
requirements; none is implied by this review submission.

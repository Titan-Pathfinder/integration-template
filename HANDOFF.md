# Ocelot integration handoff, 14 November 2026

This update enables native midpoint quotes for locked/crossed Phoenix spline
books, using the selected five-bp final-output discount. Normal-book output,
price, ranges and instructions are unchanged. No Hawkeye, local VM or RPC runs
in preparation or quoting; research-only experiments are excluded from this
standalone package. This is submitted for engineering review, not activation.

## Implementation

- `update_state` reads one supplied account batch. Normal geometry retains the
  prior conservative provider. Locked/crossed geometry prepares common-midpoint
  models, rounding up for buys and down for sells, retaining maker widths,
  offsets, expiry and risk fields. Integer midpoints share one prepared model.
- The venue keeps original accounts. Existing native risk/capacity, finance,
  full-fill and range checks remain active against the model. Invalid/missing
  dependencies or stale oracle data still fail closed and clear prior quotes.
- Quote evaluates the prepared curve, then returns
  `floor(output * 9995 / 10000)` and `price * 0.9995` only on the approximation
  path. It remains allocation-free. The discount changes the estimate, not
  fees, user balances or the actual amount delivered by Ocelot.
- The exact-input v3/CPI contract is unchanged. Routed legs use zero per-leg
  minimum and depend on Titan's final user minimum. Standalone builders require
  a positive caller minimum. The required deadline remains `i64::MAX` as agreed.
  `bounds()` remains necessary for contiguous finance-safe modeled ranges.

## Error policy requiring review

**Five bps does not guarantee that every crossed-state quote is below execution.**
Common-midpoint pricing is not Phoenix slide pricing, and modeled executable
capacity is not an independent proof of actual capacity. The default production
provider retains regressions demonstrating residual overquotes in controlled
four-tick and wider crossings. For example, an eight-tick fixture still overquotes
by 8.8924 bp after discount; the 32-tick stress sweep reaches 51.1436 bp.
These controlled mutations are not a measured live failure rate or proof of
reachability through every valid maker update sequence. No crossing-width gate
or adaptive buffer is included in the selected policy.

The unmodified captured locked/crossed SOL books pass 188,680 actual execution
comparisons with no overquotes or failures; maximum underquote is 15.8219 bp.
The controlled sweep checks 2,704,632 quote inputs and compares 52,136 executions:
11,444 wider-crossing overquotes remain, with no permissive-floor execution
failures. These are sample results, not universal safety guarantees. Titan's
final user minimum must reject insufficient actual output atomically.

See [policy, examples and reproduction](MIDPOINT_VALIDATION.md),
[machine-readable measurements](MIDPOINT_MEASUREMENTS.json),
[normal curve derivation](CURVE_VALIDATION.md),
[standalone export verification](EXPORT_VALIDATION.md) and
[source provenance](SOURCE.json).

## Requested Titan review

1. Review/accept the explicit five-bp approximation and residual-error tradeoff
   against your private harness, including locked/crossed geometry and both
   SOL3L/SOL2S mint/redeem directions. Earlier native-provider review does not
   establish correctness of this new path.
2. Verify host integration invokes the subscribed refresh on Clock-only changes
   and uses `venue.bounds()`; these behaviors remain necessary even with the
   confirmed coherent account batch.
3. Verify the production router's final user minimum/atomic rollback, including
   0.5% user slippage and a result below that floor after Ocelot succeeds.
   Our test-only guard is not Titan's production guard.
4. Confirm quote/error and refresh-budget acceptance on production hardware.
   Local quote medians are about 0.11–0.12 µs; full cached refresh medians are
   1.34–1.89 ms, p99 up to 3.67 ms. Quote p99 measures batches, not individual calls.

The source suite passes 67 reference, 42 default, 40 unchanged shared and three
program/parity checks. The route guard exercises 144 successes and 96 atomic
rollbacks across normal/locked/crossed fixtures. Timestamp-only tests give 32
+21-second successes and 16 expired controls; fixed slots do not establish
liquidity survival after wallet signing. Shared old-VM execution tests are not
claimed as passed. Runtime/ABI compatibility, delay handling and ANT's unsupported
oracle profile remain outside this update's validation. Neither PR is merged and
no routing activation or on-chain upgrade is included.

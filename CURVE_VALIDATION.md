# Conservative routing curve, 12 November 2026

This is the historical normal-book curve derivation. The November 14
[midpoint policy](MIDPOINT_VALIDATION.md) supersedes locked/crossed rejection
and discounts that model's output by five bps; the model is not a proven bound
on Phoenix slide-price execution. Normal-book behavior is unchanged.

Local follow-up to Titan's marginal-price and lot-edge findings, based on source
`a99ece550cbb73b43ae067b9d01dd277e39859ff`. This change is exported to
upstream PR #10 at `035781b7` from source `b597dddf7c5ea8bd053e7cf8507dff8ee478e84e`;
[standalone verification](https://github.com/Ocelot-fi/integration-template/blob/035781b77ea434f8c91629c3af4c99afca5ef64a/EXPORT_VALIDATION.md) records the repeated checks. Locked/crossed books still fail preparation at `best_bid >= best_ask`;
midpoint approximation, slide prices and future maker-update slots are separate work.
No program, CPI, instruction, slippage or deadline changes are included.

**Review follow-up, 12 November:** the teammate confirmed the output bound and
curve properties, including 1,483 additional mutated-state execution comparisons
without overquotes or false accepts. They found mint finance gaps in advertised
ranges and unnecessary coupling of direction-specific curve failures. Both are
addressed locally below. Their reported worst mutated-state underquote was 23.7 bp;
11.8 bp is the captured-snapshot maximum, not a universal error ceiling.

## What changes

`NativePhoenix::prepare` builds mint and redeem curves from the same conservative,
capacity-limited spline depth and effective fee used by the native matcher.
`Curve` stores a lower envelope of positive-slope affine lines. Slopes decrease
across segments, so the continuous estimate is increasing and concave. Preparation
uses depth levels, not one entry per lot or a fitted sample of quotes.

The venue first runs the existing whole-lot issuer calculation and all its
execution checks. It then returns the curve output and that segment's marginal
output/input rate. A curve above the native output bound is rejected, never clipped
back to the discontinuous output. Providers without a prepared curve fail closed
at the venue interface; the raw issuer kernel still supports them for diagnostics.

Queries use a binary search and fixed-point integer arithmetic without allocation,
network calls or VM execution. The original native fill/risk checks still run;
curve preparation does not replace them. `quote::quote` remains the unsmoothed
issuer reference. Its within-lot slope is no longer the venue's reported `price`.

A direction-specific curve error now leaves that direction unavailable and logs
its reason during preparation. The other direction can still quote. Shared
account, oracle, risk and locked/crossed geometry failures still reject the entire
refresh. This does not permit a quote without a supported curve.

The estimator anticipates adverse next-lot costs and delays favorable execution
gains. It changes the routing estimate, not what the Ocelot contract pays. The
contract executes the original whole-lot instruction and can return more.

## Bound on native output

All quantities below use integer token atoms and Phoenix lots. Define:

- `M`: marked USDC atoms per lot; `L`: target leverage; `S`: accounted shares.
- `E = gross - liability`: holder equity; `B`: absolute base position in lots.
- `Q(n)`: cumulative conservative execution notional for `n` lots.
- `f`: effective Phoenix taker fee fraction; `a = 1 - issuer_fee / 10000`.

The existing matcher charges `ceil(Q(n)*f) + n - 1` when `f > 0` and `n >= 1`.
`Q(n)*f + n` is an upper bound on that fee; zero fee needs no reserve. Including
fill-versus-mark cost gives a convex piecewise-linear cost:

```
buy:  C(n) = Q(n)*(1+f) - M*n + n
sell: C(n) = M*n - Q(n)*(1-f) + n
```

Omit the final `+n` when `f = 0`. Each prepared depth level gives a supporting
line `c*n+b` for this cost. On the supported depth, cost is their maximum. The
implementation carries coefficients as exact integer ratios with a millionth
denominator, and fails closed on arithmetic overflow or nonpositive output slope.

### Mint

Actual net deposit is `d = ceil(a*x)` and actual lots are `floor(d*L/M)`.
For `z = a*x*L/M`, the lot count lies in `[z-1, z+L/M]`. When the issuer fee is
zero, `d=x` exactly and the upper endpoint tightens to `z`.

For each cost line, maximize it over that interval: choose its upper endpoint for
a positive cost slope and its lower endpoint for a negative slope. Take the
maximum across all lines. This gives an upper cost bound `U(x)`, including both
adverse fills and favorable fills that must not be credited too early.

```
mint lower estimate = (S/E) * (a*x - U(x))
```

Use ratio 1 for a valid zero-supply bootstrap. Since `d >= a*x` and `U(x)` bounds
the actual rounded-lot cost, flooring this estimate cannot exceed the native
issued shares. Subtracting a convex upper cost gives the concave lower envelope.

### Redeem

For `z = B*x/S`, normal lot sizing lies in `[z-1,z]`. A final circulating exit
with abandoned accounted shares can round up; when that case exists, conservatively
extend the interval to `[z-1,z+1]` for the whole curve. Maximize each cost line over
the interval as above. Partial redemption does not credit positive execution
gains, so also include a zero-cost line: use `max(U(x),0)`.

```
redeem lower estimate = a * (E*x/S - 1 - max(U(x),0))
```

The atom reserve follows from
`floor(gross*x/S) - floor(liability*x/S) >= E*x/S - 1`.
The issuer's fee rounding can only raise output relative to multiplying by `a`.
Declining to credit favorable execution also bounds full exits. Cash-only
redemptions use the zero-cost line. Very small valid cash exits can be declined
when the conservative estimate rounds to zero; the synthetic tests explicitly
classify this only for native outputs of at most two atoms.

### Fixed-point rounding and domain

Round each positive slope down to Q64.64 and each intercept down to a whole atom
before building the lower envelope. This preserves the lower bound and costs
less than two output atoms relative to each unquantized line at any `u64` input.
Wide division occurs during preparation and range discovery only. Query multiplication splits high
and low 64-bit words; output is never calculated through floating point.

The continuous envelope is concave; integer outputs retain unavoidable one-atom
rounding. The tests use Titan's unchanged `1e-5` relative and two-atom chord
tolerances. `price` is the active segment slope, including execution costs. At a
kink it is the right-side slope, a valid one-sided marginal rate.

`quote(0)` remains Titan's spot-query sentinel: zero output and a positive marginal
price. The lower curve can have a negative intercept from its cost reserves.
Concavity is asserted over the positive executable domain, not across the
non-executable gap from zero to the minimum Phoenix lot.

## Contiguous redemption bounds

The unchanged shared suite exposed a separate old issue: binary-search bounds
can skip finance failures near a full exit. In `sol2s-465928707`, the old advertised
range includes input `18,938,719`, which fails `finance venue margin exceeded`.
The failure reproduces on the pre-change source and is independent of smoothing.

`bounds()` now reserves a conservative residual position for partial redemptions.
With residual fraction `t=1-x/S`, residual exposure is at most `B*t+1`, and venue
equity after withdrawal is at least `gross*t-cash`, regardless of fill cost.
For maximum venue leverage `K` in bps and minimum venue equity `V`, require:

```
t >= (cash + V) / gross
t >= (M*10000 + cash*K) / (gross*K - B*M*10000)
```

Round required residual shares up and cap by circulating supply. Nonpositive
headroom or overflow fails closed. This can exclude individually valid near-full
partial exits from the router's advertised range. Direct quotes, including full
exits, retain their original execution checks and are not cut off by this range
cap. Other risk/capacity checks remain mandatory on every query.

## Contiguous mint bounds

The same noncontiguous finance eligibility can occur on mints when current
leverage is close to its limit. A regression reproduces the review's hole at
input `866219` in `sol2s-467431351`: the old advertised range includes it, but
both the native quote and captured program execution reject it. The limit is
**20,200 bps**, as used in `review5.rs` (`target + 200`); the review prose's
22,000-bps label is a typo. The 22,000-bps control does not reproduce that hole.

Let the prepared fixed-point output envelope be `Y(x) = min_j(r_j*x + b_j)`.
Convert it back to added equity with `h=E/S` (or 1 for bootstrap). The curve
derivation establishes `h*Y(x) <= actual added equity`. Let `G` be gross equity,
`C` cash, `K` finance leverage in bps, `V=max(minimum venue equity,1)`, and
`epsilon=0` for zero mint fee or 1 otherwise. Post-mint marked exposure is at most
`B*M + a*L*x + epsilon*L`. Thus require, for **every** output line:

```
G - C + h*(r_j*x+b_j) >= V
K*(G - C + h*(r_j*x+b_j)) >= 10000*(B*M + a*L*x + epsilon*L)
```

Each inequality defines an interval of allowed input. Their intersection is
contiguous, including when deposits improve an already high leverage ratio or
push it toward a tighter limit. Solve using checked unsigned 256-bit arithmetic
with separate positive/negative terms: ceil lower endpoints, floor upper
endpoints, reject infeasible/overflowing intersections. Exact zero slopes are
handled explicitly. The venue passes this interval to its existing range search,
which retains all remaining native eligibility checks. No wide arithmetic is
added to `quote()`.

This guarantee is about finance eligibility, not a replacement for native
execution checks. Like the redemption cap, it can omit individually executable
amounts outside the conservative range. In the cited SOL2S state the advertised
mint minimum rises from `468416` to `3809792` atoms (about $0.47 to $3.81), while
the upper bound remains `3992901056`. All eight advertised captured-state ranges
remain unchanged at their original finance limits. Direct quotes outside the
range still run the original checks; valid isolated amounts are not disabled.

The bound is in the `TradingVenue::bounds()` override. Titan must confirm its
production range discovery honors that override; independently binary-searching
the raw `quote()` domain would reintroduce the same problem in both directions.

## Verification

Four public SOL3L/SOL2S snapshots, both directions:

- **273,390 accepted boundary/interior comparisons:** every lot edge in the raw
  quotable ranges, nearby atoms, seeded interiors and full exits. No downward
  output steps, increasing marginal rates, MVT violations or output above the
  original native bound. Native rejections remain rejections.
- **9,666 accepted synthetic comparisons:** both position directions, multiple
  levels, adverse/favorable fills, zero/nonzero Phoenix fees, several issuer fees,
  cash-only exits, bootstrap and abandoned supply.
- **578 accepted captured-execution comparisons:** curve breakpoints, range
  endpoints, small/large sizes and full exits; zero overquotes or failed accepted
  executions. 430 within 1 bp; maximum underquote **11.699366 bp** at SOL2S redeem
  input `761602` in `sol2s-467431351`.
- The original 205 accepted execution comparisons and 39 matching declines still
  pass. Their maximum underquote is now **11.804777 bp**, replacing the previous
  4.144072 bp statistic for this estimator. Maxima by capture: 8.240929, 7.160552,
  5.973919 and 11.804777 bp, in fixture order. These maxima are sample evidence,
  not a universal error ceiling. The largest percentage errors are tiny trades.
- **20 unchanged shared checks pass:** construction/allocation, zero spot input,
  monotone output, monotone price and mean-value theorem on each of the four
  snapshots, served through the read-only localhost fixture RPC. This is not a
  live-RPC acceptance run. Shared assertion source `tests/common/mod.rs` is unchanged.
- New locked/crossed regression confirms both still invalidate prepared quotes.
  Exact fixed-point/hull tests cover duplicate slopes, crossings and large inputs.
- Before-change reproduction: the exact Titan drop (`601134 -> 601135`), the full
  lot-edge sweep and the finance-gap regression all fail against archived HEAD
  `a99ece55`. The baseline has a renamed package to isolate Cargo artifacts.

Existing raw issuer parity, oracle/risk, subscription/cache, zero-supply,
withdrawal, routed execution, final-output rollback and deadline checks are
retained. See the commands below for the selected host suites. Test logs are local
under `.context/titan-curve-2026-11-11/`; that directory is not part of the handoff.

Review follow-up regressions are in `tests/ocelot_curve.rs` and `curve.rs`:

- Both findings fail before the fixes: the known mint hole lies inside the old
  range; a negative redemption slope aborts refresh and prevents a healthy mint.
  After the fix that mint quotes and executes, while redemption stays unavailable.
- **2,040,435 range comparisons**, with zero quote failures, output decreases,
  price increases or MVT violations inside returned ranges: four captures times
  ten finance/fee scenarios, both directions, every lot edge and adjacent atoms,
  4,096 atoms at both endpoints, plus seeded uniform/log samples.
- Scenarios include captured limits, target +20/+200 bps, current leverage
  ±50 bps, target +10%, 120% minimum equity, 20% cash, zero issuer fees and zero
  Phoenix fees. 24 of 80 stressed directions advertise no range; these are
  conservative unavailable states, not successful routing checks.
- The integer half-plane solver is independently compared with direct arithmetic
  across both slope signs, zero slopes and fractional/exact crossings, plus wide
  operands and overflow controls.
- **616 accepted execution comparisons** at stressed range endpoints, adjacent
  atoms and interior sizes all execute without overquotes. The original captured
  578/205 execution sweeps and their output-error figures are unchanged.

Follow-up logs are under `.context/titan-curve-review-2026-11-12/`. The original
review probes remain separate in `.context/titan-review/`; their findings are
recorded as reviewer evidence rather than claimed as our own reruns.

After the follow-up: **62 selected reference-feature tests, 38 default-feature
tests and all 20 shared fixture checks pass**. Both Clippy configurations pass
with warnings denied; formatting and whitespace checks pass. Shared assertion
source and the generic range-search helper remain unchanged. Program sources and
quote output arithmetic are untouched by the follow-up; program/parity evidence
and performance measurements below are from the preceding curve validation.

Before the follow-up, selected reference-feature suites passed 56 tests before adding the explicit
locked/crossed guard regression. The final default-feature run passed 34 tests,
including that regression and the full curve/hull suite. Program/enum parity:
3 passed. Default and reference Clippy selections (including the shared wrapper)
pass with warnings denied; formatting and whitespace checks pass. Benchmark and
fresh-RPC diagnostics are excluded from the broad runs and are not claimed as
passing there. The complete quote benchmark was run separately as below.

Program sources are unchanged; no SBF rebuild was needed for this host-only
change. Replay used the existing router
`b8997ff5ef34d8fa27a1e71fdfa1b134a6938788de920fa0e1d19b1f7d8652ba`
and test guard
`9f5383d9036a493f3132dfdb4090ebd479f5237f7c5d198d6fd9215436bb090c`.

## Local performance

Release build on the development host, four captured markets, no network:

| Measurement | p50 | p99 | Maximum |
| --- | ---: | ---: | ---: |
| Complete quote, 100 batches of 10,000 calls (mean per call) | 115.2 ns | 119.7 ns | 120.4 ns |
| Native preparation, 100 calls | 329.8 µs | 436.6 µs | 448.6 µs |
| Complete cached refresh, 100 calls | 1,228.0 µs | 1,397.2 µs | 1,546.8 µs |
| Sequential refresh, 4 fixture instances, 100 cycles | 5.327 ms | 8.535 ms | 10.695 ms |
| Sequential refresh, 26 fixture instances, 100 cycles | 63.904 ms | 191.552 ms | 275.109 ms |

The fan-out test repeats the four snapshots; it is not 26 distinct live markets.
It shows material host/copy/scheduling variance and is not a production slot-time
guarantee. Individual timed quotes (100,000 calls, timer overhead included) had
p50 250 ns, p99 292 ns, p99.9 417 ns and maximum 33,542 ns; timer-only p50/p99 was
42 ns. Quantiles use nearest rank. The sub-microsecond batch means are not a
worst-case latency guarantee. No preparation, account fetch or simulation is
included in quote timing; cached refresh includes in-memory cache reads/copies.

## Reproduce

From this package (prebuilt local router/guard binaries are required for routed
tests; see README for builds):

```sh
cargo +1.96.0 test --locked --features reference-simulation --lib \
  --test ocelot --test ocelot_native --test ocelot_phoenix --test ocelot_state \
  --test ocelot_replay --test ocelot_spline --test ocelot_slippage \
  --test ocelot_curve --test your_venue_creation \
  -- --include-ignored --skip benchmark_ --skip fresh_spline_comparison --nocapture
cargo +1.96.0 test --locked --lib --test ocelot --test ocelot_native \
  --test ocelot_phoenix --test ocelot_spline --test ocelot_slippage \
  --test ocelot_curve --test your_venue_creation
cargo +1.96.0 test --locked --manifest-path program-template/Cargo.toml --lib --test venue_parity
cargo +1.96.0 test --release --locked --test ocelot_phoenix benchmark_complete_native_quote -- --ignored --nocapture
```

For shared assertions, start the fixture server in a separate terminal. It serves
only captured account reads, never forwards requests or submits transactions:

```sh
python3 tests/support/fixture_rpc.py tests/fixtures/sol3l-465928618.bin.gz 18899
# In another terminal:
SOLANA_RPC_URL=http://127.0.0.1:18899 \
OCELOT_TEST_MARKET=AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM \
cargo +1.96.0 test --locked --test your_venue -- \
  --skip bound_simulation --skip random_samples --skip quoting_speed --nocapture
```

Repeat with each bundled snapshot; SOL2S uses market
`CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp`. The three filtered shared tests
are not reported as passing: execution comparisons use the compatible reference
runtime, and speed uses the complete native benchmark. The original shared VM
cannot load current Phoenix executables and its exact-output policy does not
establish acceptance of a conservative estimate.

## Remaining scope

Titan engineering re-review includes the mint-range and direction-isolation follow-up. Locked/crossed availability,
future maker-update slots, live-state coverage and Titan's acceptable output
error remain open. No common-midpoint approximation is present. Same-state
underquoting does not guarantee execution after signing delay. Titan production
router minimum/rollback and Clock-only refresh delivery still need verification.
ANT3L/ANT2S remain outside the validated profile; nothing here activates routing.

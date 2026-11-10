# Native quoting implementation, updated 10 November 2026

**A conservative native Phoenix provider is implemented and installed by default.**
`OcelotVenue::update_state()` prepares it from cached accounts; `quote()` runs
native integer arithmetic without RPC, VM execution or heap allocation. Supported
snapshots now initialize and construct transactions without an injected test matcher.
This is a local review candidate, not Titan acceptance or live distribution. The
November 10 candidate is supplied here for Titan engineering review.
The dated checkpoints below record internal history before this standalone export.

## Review direction

Titan will review conservative estimates and rerun speed and simulation comparisons.
Its 1 microsecond target is a heuristic with possible case-specific leeway; no
alternative ceiling or output-error tolerance has been agreed. Jarry confirmed
spline-only pricing. Gally's SDK link is not a dependency: the checked public
Rise v0.9.3 revision is `2ce47221ef04a8d5879d39d31e82f9e92021753e`.

## Prepared provider

`src/ocelot/phoenix.rs` implements `NativePhoenixFactory` and `NativePhoenix`.
Preparation validates account owners, discriminators, identities, market status,
trader capabilities, cluster restart status, custody, and the supported account layouts.
All calculations use the supplied state, including actual fee overrides and risk data.

- **Spline prices:** Decode base widths, region offsets, visible fills and inclusive
  expiry; prepare cumulative depth. Hidden take liquidity is excluded from quoted
  taker liquidity. Unlike the earlier diagnostic, production follows the SDK's
  saturating expiry even for future-anchored entries, so they cannot be silently
  omitted from the slide-price check.
- **Maker capacity:** Read live collateral and positions from the global trader
  index and active buffer. Set conservative margin from all supported perp positions,
  including funding debits/credits and negative PnL, with no positive PnL or spot
  collateral credit. Apply each spline's collateral haircut and position limits.
  Additional exposure stays inside the first leverage tier. Existing margin gets
  no reduction credit; adverse execution reserves are charged on top. Both spot
  and perp components require a fresh, nondivergent oracle quorum regardless of
  their pricing weights. A nonzero-weight book component must also be fresh under
  its cached threshold. A fresh aggregate mark alone does not establish
  withdrawal eligibility; soft-stale margin/withdrawal rules are not modeled.
- **FIFO uncrossing:** Never count FIFO liquidity toward quoted output. For each
  spline, reserve all FIFO lots that could cross its mid, independently of other
  makers, including orders that might expire or be rejected. Remove those lots
  from its front and reserve their margin and position capacity. This intentionally
  understates liquidity without reproducing Phoenix's pro-rata/hidden-take matcher.
  Crossed spline geometry requiring slide prices rejects preparation.
- **Taker and fees:** Model the child position, funding settlement and partial/full
  close cost basis. Read the book taker fee and child's override; round the effective
  rate upward and reserve up to one additional atom per lot for split-fill fee
  rounding. Check conservative post-fill margin and open-interest room, including
  a reserve for potential FIFO uncrossing.
- **Withdrawals:** Reserve remaining position margin without crediting positive
  unrealized PnL. Reserve partial-close cost-basis dust. Bound withdrawal by child
  collateral, Phoenix's replenished immediate-withdrawal budget and queue state,
  Phoenix token-vault balance and Ember USDC-vault balance. Ocelot's own finance
  margin and global/market throttles are separately enforced by the issuer kernel.
  Capacity must cover better actual execution too: partial exits reserve the full
  pro-rata claim, and full exits use a favorable-price upper bound with zero fees.
  A regression sets the budget between estimated and actual payouts and confirms
  that both the quote and executable withdrawal reject it.

The public SDK supplies no Ember account decoder. This implementation accepts
only the exact static 136-byte configuration validated by all four captures
(SHA-256 `3422253fc2e99823d0fb5292912847e27ed175215d2bb770364b79c321b18bc9`).
Any configuration change requires review. Vault balances remain live cache inputs.
No dependency on a local `.context` checkout is needed to build the package.

Phoenix's per-action validity rules and oracle policy are also pinned. SHA-256
over metadata bytes `352..608` followed by `808..813` must equal
`afd967552f05e617f90d8eb493daa305b9f099c1efe87015a394d9bbb1f8b6df`.
This is the policy in all four SOL captures: divergence radius 500, minimum one
response, and both hard-stale multipliers zero. Other policy profiles fail closed;
makers with nonzero positions using them contribute no liquidity. The live
divergence scores are **not** pinned: each oracle must have score zero to count
toward spot/perp quorum. The independent book component has no oracle score.
Weights and stale thresholds remain live inputs, outside the policy hash. A zero
spot/perp weight does not bypass validity checks; only the book can be disabled
by weight under the pinned rules. This is covered by executable regression tests.
The SDK supplies these layouts but not the per-action validity evaluator; this
guard is a conservative restriction, not an implementation of that evaluator.

## Supported scope and conservative refusals

The provider supports the current classic SPL USDC/LT issuer path: a cold,
single-perp isolated child, an empty cold deposit parent, regular non-finalized
Phoenix perp pricing, zero global withdrawal-margin-factor override and one
index/buffer arena account each. It refuses hot children, unknown layouts,
conditional-order children, pending child withdrawals, special commodity/after-hours
price rules and crossed splines. Makers with unknown/locked-reserve extensions,
stale or special-priced nonzero positions, or FIFO orders contribute no liquidity.
Spot balances are ignored rather than added to capacity. Expanded arenas reject.
Non-bootstrap mints below 10,000 LT output atoms decline, bounding the on-chain
share-rounding check even when execution is better than the estimate.

**ANT launch coverage is not established.** In both November 9 captures (slots
467430913 and 467431351), the Phoenix `ANTHROPIC` entry has policy hash
`7b047ebde93b9ee45e84d217b60493e2e5a0a95b44e0527318b94149c76dc10b`, with both
hard-stale multipliers 10 rather than SOL's zero. Its special-price status byte
at metadata offset 1456 is also 5 rather than zero. Either condition is outside
this provider's supported target state. These are captured observations, not a
new live account read or an executed ANT quote. Before enabling ANT3L/ANT2S,
capture their current configured accounts and validate oracle policy, special
pricing and mint/redeem execution. Adding another hash alone is insufficient.

These restrictions can decline trades that Phoenix would execute. They do not
claim to duplicate the full Phoenix matcher. Supported-snapshot agreement is
measured evidence, not a proof covering every reachable chain state or a guarantee
after state changes. Titan must assess both error and availability on more live states.

Titan must refresh the spline, book, index, buffer, full perp map, child/parent,
withdrawal queue, vaults, configuration, Ocelot finance/fee/throttle accounts and
clock together. `AccountsCache` has no per-account slot/coherence contract, so the
integration host must supply coherent snapshots and invalidate them promptly.
Discovery reads are discarded. All decoded quote fields, limits and the native
provider now come from one final complete `get_accounts` result. A dependency
change during discovery fails the refresh. This removes internal mixing across
reads; it cannot detect a torn batch returned by the host. Titan must supply a
coherent final batch, including its clock. A failed refresh clears prepared state;
a missing refreshed account cannot retain its prior value. All requested keys
remain subscribed even when absent, so creation of an optional market throttle
triggers a refresh. The market throttle and Phoenix withdrawal queue are discovered
without requiring a redeem instruction to build, including at zero supply.
Queries perform no freshness RPC. Output, deadline, input-amount and
finance-revision guards remain in the transaction.

## Issuer math and provider contract

`src/ocelot/quote.rs` applies the issuer fee, sizes native lots, prices issued shares
from execution-adjusted equity after streaming liabilities, and applies redemption
costs, liabilities and fees. Final circulating redemptions retain the existing lot
round-up rule. Finance, supply, deposit/equity caps and share-rounding checks remain.
The instruction builder shares the same lot-sizing function.

`PhoenixQuoteFactory::prepare(&State)` creates an immutable provider for that state.
`set_phoenix_factory()` remains available to override the bundled implementation.
`Fill` may describe exact execution or a conservative execution model. Its notional
and fee together must not overstate post-order equity; its collateral and available
withdrawal must support the resulting issuer payout. A full close must reconcile
modeled collateral and equity. Separate collateral-upper-bound and immediate
liquidity-capacity fields prevent the lower output estimate from understating
the amount the contract might actually withdraw. Estimated fees are explicitly not called actual fees.
No partial input or partial Phoenix fill is accepted.

`price` remains the local within-lot slope with integer rounding relaxed. Lot-boundary
steps remain; it is not a globally concave curve or a derivative at a jump. Titan's
original shared curve assertions remain unchanged and are not reported as passing.

## Complete-provider evidence

`tests/ocelot_phoenix.rs` exercises the default provider through `OcelotVenue::quote`.
Across four captured SOL3L/SOL2S states, **205 accepted quotes executed successfully
with zero observed overquotes**. The sweep includes both directions, lot boundaries,
small trades, larger/multiple-level fills and full circulating exits. There were
39 declined inputs, including minimum-lot and supply violations. Every one is now
asserted to reject in the reference executable too. This classification applies to
these captures, not all supported or deliberately refused states.

| Snapshot | Accepted comparisons | Declined inputs | Largest underquote |
| --- | ---: | ---: | ---: |
| SOL3L / 465928618 | 58 | 3 | 0.033367 bp |
| SOL2S / 465928707 | 45 | 16 | 0.022224 bp |
| SOL3L / 467430913 | 58 | 3 | 4.144072 bp |
| SOL2S / 467431351 | 44 | 17 | 0.023104 bp |

The November 10 policy guard excludes some makers with other-policy positions.
Five larger redemptions in SOL3L / 467430913 now underquote by more than 1 bp;
the other 200 accepted comparisons remain within 1 bp. The widest is the full
3,448.654926 SOL3L exit: estimated 3,239.360861 USDC versus executed 3,240.703832
USDC (4.144072 bp, about 0.0414%). An isolated diagnostic retaining divergence
checks but removing only the policy pin restores the previous 0.034725 bp maximum.
Keep the guard until additional profiles are validated; Titan must assess this
availability/accuracy trade-off. No 1 bp acceptance ceiling has been agreed.

Additional tests compare fee-rate/override changes and immediate-withdrawal behavior
against the captured executables. They check maker collateral and position caps,
haircuts, capability changes, exhausted withdrawal budgets/vaults, malformed arenas,
changed Ember configuration, missing accounts and refresh invalidation. Allocation
checks cover successful and rejected quotes. The selected host suite passes 46 tests, plus 3 program-host/enum-parity tests.
A separate default-feature run passes 26 tests; both Clippy configurations pass.
Routed replay uses the bundled provider across all four captures, $10 and $1,000
mints, available larger redemptions and full exits: 51 routed executions across
17 input/direction/state cases and three reserved-balance modes. Direct/routed
execution output must match exactly, with the native quote at or below it.

The November 10 release benchmark at `d881a120` reports nearest-rank percentiles and maximum
separately. One million complete quotes measured batch-mean p50 **96.6 ns**, p99
**479.1 ns**, max **543.6 ns**, in 100 batches of 10,000 calls. In 100,000 individually
timed calls, p50/p99/p99.9 were **125/167/250 ns** and max **39,000 ns**, including
measurement overhead and OS scheduling; the timer-only p99 was 42 ns.

Native preparation p50/p99/max was **353.9/406.4/437.1 µs**; complete cached refresh
was **1,270.7/1,486.3/1,511.3 µs**, over 100 calls. The complete final read, validation,
component eligibility checks and policy hashing add refresh work, while the quote kernel is unchanged. Sequential
refresh of 4 fixture instances measured **4.976/5.744/5.937 ms**, and 26 instances
**32.840/37.435/47.931 ms**, over 100 cycles each. Those instances repeat the four
SOL fixtures; they are a fan-out cost measurement, not 26 distinct live markets.
No network delivery is included and no shared-decoder optimization is assumed.
Titan's update frequency and refresh budget remain open.

Earlier reports used the last of 100 sorted observations as "p99"; those figures
were maxima. Historical complete-quote p50/max was 106.5/139.7 ns, and refresh
877.2/1,621.1 µs. The corrected November 9 p50/p99/max was 106.8/149.9/150.0 ns
for quote batch means and 1,202.7/1,423.0/1,489.5 µs for complete refresh.
These local measurements are not production latency guarantees;
quote latency and refresh latency are separate budgets.
The zero-weight follow-up changes preparation only; these timings were recorded
before that follow-up, and the quote kernel remains unchanged.

## Review hardening, 9 November

- `tests/ocelot_state.rs` advances discovery to an actual executed mint's post-state,
  for all four captures and both cold/warm initialization. Every decoded field and
  both quote directions must match the coherent final state; quotes are compared
  against its executable. Missing final accounts and changed dependencies invalidate
  an earlier usable venue. Both new tests fail against the pre-fix `67da3b7e`
  adapter in an isolated copy (with only reference capture tooling added).
- Execution coverage now includes backed nonzero finance cash for partial/full
  redemptions, full-exit withdrawal budgets between estimated and actual output,
  and partially binding maker collateral, haircut and position caps. The maker
  tests retain executable small trades while declining $1,000 trades; they compare
  eight accepted constrained quotes against execution. Full-exit tests also show
  the deliberately conservative refusal possible at exactly the actual budget.
- Expiry tests retain the original encoded transaction, output floor and deadline,
  then advance only the clock by 0, 4, 5, 6, 12 and 19 slots. Of 126 attempts,
  **106 settle above their original floor**, including **31 below their original
  estimate**, and **20 reject**. Re-prepared accepted quotes remain below execution.
  All 21 original transactions reject after their deadline. An additional control
  renews maker and oracle timestamps with all prices fixed, then changes only
  the expired deadline to show that it blocks otherwise executable transactions.
  Renewing makers alone at +51 slots still fails withdrawals because the oracle
  inputs are stale. The native provider now rejects that state too, using cached
  component thresholds and minimum-response requirements. These freshness checks
  run during preparation; the host must still trigger clock-only refreshes.

The expiry test quantifies a remaining integration policy decision; it does not
make old quotes conservative after market changes. No landing horizon, per-slot
host callback guarantee or replacement for the reference 1% / 20-second settings
has been agreed. In particular, the deadline is a Unix timestamp, not a last-valid
slot. Do not convert an assumed slot duration into a claimed on-chain slot bound.
Titan must confirm snapshot consistency, clock-only refreshes, quote-to-execution
latency, and slippage/deadline policy before activation. No upstream PR update or
production deployment is part of this hardening.

## Follow-up review fixes, 10 November

The second review of `501a6c82` confirmed that the cache-consistency fix holds,
but found accepted quotes that Phoenix rejects in the same supplied state.
The previous oracle-age check was incomplete: it counted divergent responses and
accepted arbitrary changes to action-specific price validity rules. The reviewer
reported 54 false accepts in 210 oracle-mutation comparisons, with no overquotes.
Their separate 1,609 maker-mutation comparisons found neither failure mode; those
are reviewer results, not a new exhaustive production-state guarantee.

All three reported defects are fixed locally:

- Oracle quorum counts only fresh responses with divergence score zero; policy
  changes fail closed under the hash above. Tests execute 32 accepted quotes with
  one/two divergent responses, confirm 16 all-divergent execution rejections and
  four mixed fresh-divergent/stale-eligible withdrawal rejections. Four executions
  also reproduce the changed order-rule rejection. Nine representative policy
  edits across four captures all invalidate preparation and clear prior quotes.
- `State` retains the entire requested key set, including absent accounts. Tests
  record every discovery/final read across all four captures and check that each
  key remains watched. Both SOL2S captures exercise absent throttle → enabled
  1-USDC throttle → absent → enabled, refreshing only when the changed key is
  subscribed. Quotes agree with actual execution, including `WithdrawRateLimited`.
- Withdrawal-queue and market-throttle discovery is unconditional. The regression
  clears fees/liabilities before a full SBF-executed redemption, then uses its real
  post-state with zero supply, accounting shares, position, collateral and equity.
  Cold and warm initialization both support an executed $10 bootstrap mint across
  all four captures. This strengthens the review's supply-only mutation, which
  proved missing discovery but did not represent a fully emptied market.

All six new tests fail against unchanged `501a6c82` production source in an isolated
copy and pass with these fixes. The full selected host suite, routed replay,
default-feature suite and both Clippy configurations pass; counts are above.
Router source and its SBF artifact are unchanged. No RPC-dependent upstream
acceptance suite or live transaction was run. The owner requested committing these
fixes for teammate re-review on the internal review branch. Public fork PR #10 remains at the
previous submission; export follows internal review. No merge or activation is included.

Ask Phoenix: **Can you share the Rust evaluator for per-action price validity,
including divergence eligibility and soft/hard-stale handling, for use against
cached account data?** This would help expand supported policies; it is not needed
to ship the conservative guard for review. Titan's coherent-cache, clock-only
refresh, quote-lifetime and slippage/deadline questions remain unchanged.

## Zero-weight follow-up, 10 November

The teammate's re-review of `d881a120` closed absent-account subscriptions and
zero-supply discovery. Their original oracle mutations now produced no false
accepts, but a new case exposed a remaining validity gap: a zero spot/perp weight
skipped freshness even though Phoenix still required that component to be valid.

The narrow fix checks spot/perp freshness and divergence regardless of weight;
the pinned policy and live thresholds are unchanged. The zero-weight book
exception is retained and verified against execution. The new
`zero_weight_oracles_still_require_freshness` test fails on `d881a120` and passes
with the fix. Across all four captures, both directions and 1/5-unit inputs, it
covers fresh components, one slot past the cached threshold and 500-slot age:
80 accepted cases execute without overquoting, 48 refusals match execution
rejections, and 16 are intentional conservative declines of executable trades.
Every refused refresh invalidates the previous prepared venue.

The selected host suite now passes 46 tests; the 26-test default-feature suite,
3 program/parity tests, 51 routed executions and both Clippy configurations pass.
The original 205 comparisons, 39 matching declines and 4.144072 bp maximum
underquote are unchanged. Router code and artifact are unchanged. The owner
requested committing this additional fix for teammate re-review on the internal
review branch; public fork export follows internal review. Titan's cache,
clock-refresh and quote-lifetime decisions remain open. Captured ANT limitations
are recorded above; ANT support is a separate validation step before activation.

## Internal review sign-off, 10 November

The teammate's final re-review of `a8c5f9fd` closed the last blocker and found no
new blockers for quote correctness and execution safety against the same supplied
state. They reproduced the zero-weight regression's 80/48/16 split, confirmed
zero false accepts in the formerly failing stale-oracle cases, and found no
overquotes or false accepts across another 336 oracle-field comparisons. Throttle
lifecycle, zero-supply transitions and mixed-cache reads remained correct. They
also repeated the 46 host tests, 3 program/parity tests, 51 routed executions and
default-feature suite, with unchanged router hash and 205-comparison error figures.
These are reviewer-reported reruns; no additional code changes followed sign-off.

One non-blocking availability restriction remains: a weighted book older than its
cached threshold (200 slots in these captures) makes this provider decline even
though Phoenix can execute under the pinned rules. The zero-weight book exception
is correct. Spot/perp age exactly equal to threshold is another deliberate
conservative refusal. Discuss quiet-market availability with Titan before deciding
whether to relax these checks; this review does not change them.

This clears the candidate for Titan engineering review, not live activation.
Titan must confirm final-batch coherence, clock-only refreshes, quote lifetime and
acceptable output error, alongside the routing/runtime requirements in the handoff.
The prior delayed-landing rerun reported 172 executions at/above the quote, 84
below the quote but above the output floor, and 64 rejections; none settled below
the floor. The output floor is an execution guard, not an acceptable quote-error
target. ANT configuration and special-pricing validation remain separate scope.

Reproduce from this package:

```sh
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_phoenix --test ocelot_state -- --nocapture
cargo +1.96.0 test --release --locked --test ocelot_phoenix benchmark_complete_native_quote -- --ignored --nocapture
cargo +1.96.0 test --locked --features reference-simulation --test ocelot_replay -- --include-ignored
```

Routed replay requires the existing template router SBF artifact; see README.
`reference-simulation` is opt-in and only used for verification. Default library
builds have no LiteSVM dependency, and there is no simulation fallback in quoting.

## Spline experiment, 9 November

Historical component evidence remains in `tests/ocelot_spline.rs`: a geometry-only
bridge matched 151 exact outputs and 6 matching rejections across four captures;
its earlier 74.3/89.1 ns batch-mean p50/max excluded production risk and withdrawal
checks. That bridge is test-only and is not installed by the adapter. The complete
provider results above supersede it as integration evidence.

The current spline layout includes `bid_base_width` and `ask_base_width` at offsets
1112/1120. Older vendored readers omit them and would misprice these captures.
Historical vendor/audit copies were preserved.

The November 9 fixtures contain public state and executable programdata from one
RPC response per market:

| File | Slot | SHA-256 |
| --- | ---: | --- |
| `sol3l-467430913.bin.gz` | 467430913 | `3179197dd986274ceac0bc47fbed82a5183d3c7fc1bdd8193f1bc85e35262997` |
| `sol2s-467431351.bin.gz` | 467431351 | `931256a74058c8d37850ffeedfd62c2a969a0ca4a46832420e403c4e62d654f3` |

References: [pinned spline layout](https://github.com/Ellipsis-Labs/rise-public/blob/2ce47221ef04a8d5879d39d31e82f9e92021753e/rust/accounts/src/spline_collection.rs),
[Phoenix spline risk and uncrossing rules](https://docs.phoenix.trade/phoenix/matching-engine/spline-liquidity),
[pinned SDK risk math](https://github.com/Ellipsis-Labs/rise-public/blob/2ce47221ef04a8d5879d39d31e82f9e92021753e/rust/math/src/margin_calc.rs),
[price-component layout](https://github.com/Ellipsis-Labs/rise-public/blob/2ce47221ef04a8d5879d39d31e82f9e92021753e/rust/accounts/src/perp_asset_map/price.rs).

Next handoff: export this candidate and fixtures to the fork PR, have Titan review
supported-state refusals and rerun its own error/timing comparisons, then agree
runtime/ABI, exact-input routing, slippage/deadline and activation requirements.
Neither an external PR update nor production activation was performed in this step.

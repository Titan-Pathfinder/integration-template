# Titan route slippage, 10 November 2026

Titan confirms that the frontend/API caller chooses slippage and its production
router enforces the final minimum received. It permits zero minimum on the Ocelot
leg. This submission adopts that contract; it is not production activation.

## Adapter behavior

- `TradingVenue::generate_swap_instruction` still checks native quote eligibility,
  then encodes a zero venue minimum. There is no longer a fixed 1% per-leg floor.
  This method is for a protected Titan route, not standalone submission.
- `build_with_minimum` continues to require a caller-selected positive minimum for
  standalone use. The low-level encoder and CPI adapter accept a zero minimum.
- Exact-input binding, v3 discriminator/length validation, full-fill Phoenix IOC,
  finance revision and 100 bps IOC band remain in force.
- Following the subsequent Titan agreement, both adapter builders encode
  `i64::MAX` in the required deadline field. This removes the 20-second cutoff;
  transaction expiry belongs to Titan/frontend/wallet and Solana. The program's
  deadline check is unchanged. Zero is not a disable value: it is already expired.
- No pricing, cached depth, native quote math or Ocelot vault program changes.

The supplied template route has no user-selected final-output guard. Sending its
unprotected zero-minimum route would not enforce user slippage. Titan must wire
this adapter into its production guard before activation.

## Local evidence

`tests/ocelot_slippage.rs` verifies zero venue minimum in both directions across
all four SOL3L/SOL2S captures, standalone zero rejection, positive-floor encoding,
maximum deadline, unchanged revision, Clock subscription and zero-input rejection.

The routed regression uses actual captured Ocelot/Phoenix/Ember executables and a
rebuilt template router. A separate test-only SBF instruction checks the change
in the user's output balance after the route, in the same transaction. This is
a local model of the final check, not Titan's production router or a production
guard implementation. Its starting balance is supplied by the harness.

The 120 transactions cover four captures, mint/redeem, new/empty/prefunded router
accounts, a nonzero user output balance, minimums one atom below/at/above actual
output, and illustrative original-quote floors at 0.5% and 2% tolerance. The old
quote is synthetic and is not evidence of a particular market-move distribution.

- 72 successful transactions deliver exactly the expected output delta.
- 48 fail specifically at the final guard after logs confirm successful Titan,
  Ocelot and Phoenix execution. Every writable route account is byte-for-byte
  restored, including wallet balances, mints, issuer and venue state and newly
  created accounts. The payer's transaction fee is the sole allowed difference.
- Successful controls demonstrate that Phoenix state actually changes.
- The earlier standalone delay regression explicitly encodes its original 1%
  floor and 20-second deadline;
  historical results are not silently reinterpreted as zero-minimum protection.

The deadline regression advances only the Unix timestamp by 21 seconds while
holding the slot and account state fixed to isolate the removed cutoff. All eight
fixture/direction cases execute directly and through the router (16 successes),
with identical output. Re-encoding an expired deadline rejects all eight controls
with `DeadlineExpired`. This does not claim that Phoenix liquidity survives a real
21-second delay: the earlier expiry suite separately advances slots, and the local
VM disables blockhash verification. Normal transaction expiry is not bypassed by
this adapter change.

Build and run commands are in [README](README.md#verify-without-rpc). The regression
is explicitly ignored until both SBF artifacts are built; missing files fail it.
Shared upstream quote/router acceptance assertions remain unchanged.

## Production acceptance request for Titan

Run the updated adapter through the production router or an equivalent private
harness, in both mint and redeem directions:

1. With a user-selected 0.5% tolerance, confirm the signed final minimum uses the
   original route quote, correct token units, rounding and fee treatment.
2. Make Ocelot execute successfully but return below that minimum. Confirm the
   production final check rejects, rolls back Ocelot and Phoenix state, and leaves
   input/output balances unchanged except transaction fees. Include a user with
   existing output tokens so an absolute balance cannot hide the shortfall.
3. Confirm output equal to and above the minimum succeeds; include a tolerance
   greater than 1% to show the removed Ocelot floor no longer overrides the user.
4. Exercise only route graphs supported by the adapter's exact-input restriction;
   intermediate amount changes and repeated visits require separate agreement.

Also verify Clock-only subscriptions call `update_state` and expire prepared
liquidity. Titan has confirmed that is the sole venue-update callback. Local
tests do not establish delivery by its host.

Still open: production router/ABI and route acceptance, Clock-only host delivery,
measured quote-error/performance
acceptance, and separate ANT configuration/execution validation. User-selected
slippage does not bound quote age or guarantee that Phoenix liquidity survives.
Titan agreed to remove the additional deadline; this agreement does not promise
automatic re-simulation or rebuilding after slow wallet signing.

# Titan V3 Route Program — Quay venue

Exact-in Anchor route program that Titan's router uses to execute routed swaps,
with the Quay venue CPI adapter wired in.

- `initialize` creates the TitanPDA route signer.
- `swap_route_v3` takes custody through the TitanPDA, validates the venue CPI
  accounts and route-leg serialization, and CPIs into each leg's venue adapter.
- `instructions/venues/quay.rs` is the Quay venue adapter — it serializes Quay's
  exact-in `swap` instruction and forwards the route-assembled account metas.

To see the integration status, run `make scorecard` from the repo root.

## Build

```bash
cargo check --manifest-path program-template/Cargo.toml
make build-program
```

## Route Instruction Interface

The entrypoint keeps the single-byte discriminator and exposes only the fields
needed to exercise Titan's router account layout:

```rust
#[instruction(discriminator = [42])]
pub fn swap_route_v3<'info>(
    ctx: Context<'_, '_, 'info, 'info, SwapRouteV3<'info>>,
    amount: u64,
    mints: u8,
    swaps: Vec<SwapSpecInputV2>,
) -> Result<()>
```

This program models exact-in execution only: `amount` is the exact input the
router spends.

## Remaining Accounts Layout

`swap_route_v3` expects remaining accounts in this order (three optional route
slots precede them; pass this program id for any unused optional slot):

```text
[0..mints]         TitanPDA token accounts, one per route mint
[mints..2*mints]  mint accounts, aligned with the ATAs above
[2*mints..N]      venue CPI accounts for each swap leg
```

For each swap leg:

- `n_accounts` is the number of venue accounts for that leg, **including** the
  venue program id as the final account.
- The router passes all `n_accounts` accounts to `invoke_signed`, and only the
  first `n_accounts - 1` as `AccountMeta`s to the venue module.

The off-chain builder `swap_route::build_swap_leg` (root crate) assembles these —
clearing the TitanPDA signer flag, appending the venue program id, and setting
`n_accounts`.

## The Quay venue adapter

`instructions/venues/quay.rs` builds Quay's `swap` instruction and forwards the
metas; the program dispatches `Venue::Quay { sell_base }` to it:

```rust
pub const PROGRAM_ID: Pubkey = pubkey!("QUayE6nexQWYNZAEqfN8FxoNwQDSu3CAzT2qq9J1ArG");

pub fn swap(
    sell_base: bool,
    amount_in: u64,
    account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
    let side: u8 = if sell_base { 0 } else { 1 };
    let mut data = Vec::with_capacity(1 + 8 + 8 + 1);
    data.push(0x20);                                  // swap discriminator
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());      // min_amount_out — route-level slippage
    data.push(side);
    Ok(vec![Instruction { program_id: PROGRAM_ID, accounts: account_metas.to_vec(), data }])
}
```

`tests/venue_parity.rs` guards that the off-chain `swap_route::Venue` and the
on-chain `state::Venue` enums serialize identically.

## Swap simulation test

`tests/quay_route.rs` executes swaps through `swap_route_v3` using the off-chain
builder and checks the simulated output against the venue's quote in every
declared direction (shared harness in `tests/common/mod.rs`). It **skips** unless
its prerequisites are present: `SOLANA_RPC_URL`, the built program at
`target/deploy/titan_v3_venue_template.so` (`make build-program`), and a dump of
the Quay program (auto-dumped into `program-dumps/` on first run).

```bash
make build-program
SOLANA_RPC_URL=<mainnet-rpc-url> \
  cargo test --manifest-path program-template/Cargo.toml --release --test quay_route -- --nocapture
```

Or run it from the repo root with `make test-venue`.

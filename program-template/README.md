# Titan V3 Venue Program Template

Standalone exact-in Anchor template for integrating a venue CPI adapter into
Titan's router program.

This template focuses on the venue integration surface:

- `initialize` creates the TitanPDA route signer.
- `swap_route_v3` validates venue CPI accounts, TitanPDA custody, and route-leg
  serialization in the same shape Titan's router expects.
- `instructions/venues/raydium_amm.rs` is a real runnable Raydium AMM CPI example.
- `instructions/venues/template.rs` is the placeholder adapter to replace or
  rename when adding your own venue.

**To add a venue, follow the checklist in
`programs/titan-v3-venue-template/src/instructions/venues/README.md`.** To see
what's still left to fill in, run `make scorecard` from the repo root.

## Build

```bash
cargo check --manifest-path program-template/Cargo.toml
make build-program
```

## Route Instruction Interface

The entrypoint keeps the single-byte discriminator and exposes only the fields
needed to exercise Titan router account layout:

```rust
#[instruction(discriminator = [42])]
pub fn swap_route_v3<'info>(
    ctx: Context<'_, '_, 'info, 'info, SwapRouteV3<'info>>,
    amount: u64,
    mints: u8,
    swaps: Vec<SwapSpecInputV2>,
) -> Result<()>
```

This template only models exact-in execution: `amount` is the exact input amount
the router will spend.

## Remaining Accounts Layout

`swap_route_v3` expects remaining accounts in this order:

Fixed accounts include three optional route slots before `remaining_accounts`.
Pass this program id for any unused optional slot.

```text
[0..mints]         TitanPDA token accounts, one per route mint
[mints..2*mints]  mint accounts, aligned with the ATAs above
[2*mints..N]      venue CPI accounts for each swap leg
```

For each swap leg:

- `n_accounts` is the number of venue accounts for that leg.
- `n_accounts` must include the venue program id as the final account.
- The router passes all `n_accounts` accounts to `invoke_signed`.
- The router passes only the first `n_accounts - 1` accounts as `AccountMeta`s to the venue module.

The off-chain builder `swap_route::build_swap_leg` (in the root crate) assembles
these for you — clearing the TitanPDA signer flag, appending the venue program id,
and setting `n_accounts`.

## Swap Simulation Test

The template ships a LiteSVM integration test that executes swaps through
`swap_route_v3` using a venue's off-chain builder and checks the simulated output
against the venue's quote, in every declared direction. Two entry points run the same
shared suite (`tests/common/mod.rs`):

- `tests/example_route.rs` — the Raydium AMM reference.
- `tests/your_venue_route.rs` — your venue (fill in its pool + program id).

They **skip** unless their prerequisites are present: `SOLANA_RPC_URL`, the built
program binary at `target/deploy/titan_v3_venue_template.so` (from `anchor
build`), and a dump of each venue program (auto-dumped into `program-dumps/` on
first run).

```bash
make build-program
SOLANA_RPC_URL=<mainnet-rpc-url> cargo test --manifest-path program-template/Cargo.toml --release --test example_route -- --nocapture
```

Or run the example and your venue suites separately from the repo root with
`make test-example` and `make test-venue`.

## Raydium AMM Example

`raydium_amm.rs` is intentionally simple and real, and shows the exact
responsibility of a venue module: serialize the CPI instruction data and forward
the account metas in the order produced off-chain.

```rust
pub const PROGRAM_ID: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

pub fn swap_base_in_v2(
    amount_in: u64,
    account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
    let mut data = Vec::with_capacity(17);
    data.push(16);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());

    Ok(vec![Instruction {
        program_id: PROGRAM_ID,
        accounts: account_metas.to_vec(),
        data,
    }])
}
```

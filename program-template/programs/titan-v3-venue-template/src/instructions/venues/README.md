# Venue Modules

This directory contains the venue CPI adapters called by Titan's router program.
Titan routes exact-in swaps only.

## Common-Path Interface

The common path is a pure CPI adapter: serialize your exact-in swap instruction
from the provided amount, account metas, and any CPI parameters stored on the
`Venue` variant.

`zero_for_one` below is only an example parameter. Add or remove parameters as
your CPI requires, but every value must come from the `Venue` enum variant in
`state.rs` and the mirrored route-builder `Venue` variant.

For example:

```rust
pub fn swap(
    zero_for_one: bool,
    amount_in: u64,
    account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>>
```

Your function should:

- serialize your AMM's exact-in swap instruction data;
- use `amount_in` as the exact input amount;
- use the `Venue` variant fields for CPI-specific data such as direction flags;
- use your real venue program id;
- forward `account_metas.to_vec()` unless your program requires extra metas;
- return one or more `Instruction`s.

Titan's router program handles token custody, TitanPDA signing, and output settlement.
If your venue needs account data, temporary token accounts, custom signer
behavior, or pre/post CPI token movement, call that out in the integration notes.

## Add-Venue Checklist

1. Replace `template.rs` with your venue adapter, or rename it to `<venue>.rs`
   and remove the old placeholder.
2. Replace `TEMPLATE_PROGRAM_ID`.
3. Replace `SWAP_DISCRIMINATOR` and instruction data serialization.
4. Add `pub mod <venue>;` in `mod.rs`.
5. Add a `Venue::<VenueName>` variant in `state.rs`, including every CPI
   parameter your adapter needs.
6. Add a `perform_cpi_swap` match arm in `swap_route_v3.rs` and pass those
   variant fields into your adapter.
7. Mirror the new variant in the off-chain route-builder `Venue` enum
   (`src/swap_route/mod.rs`) — same name and position — and map your
   `PoolProtocol` to it in `protocol_to_venue`. `tests/venue_parity.rs` checks
   the program and route-builder enums stay byte-identical.
8. Build each route leg off-chain with `swap_route::build_swap_leg`: it calls
   your venue's `generate_swap_instruction`, clears the TitanPDA signer flag,
   appends the venue program id, and sets `n_accounts` for you. Encode the
   instruction data with `swap_route::encode_swap_route_v3_data`.

## Venue Variant Fields

If direction is not encoded in account ordering, keep a direction field on the
enum variant. Use the same pattern for any other CPI parameter:

```rust
pub enum Venue {
    MyVenue { zero_for_one: bool },
}
```

Then thread it into your venue function:

```rust
Venue::MyVenue { zero_for_one } => {
    my_venue::swap(zero_for_one, amount, account_metas)?
}
```

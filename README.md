# Dr. Fraudsworth — Titan Pathfinder Adapter

Titan [`TradingVenue`](https://github.com/Titan-Pathfinder/integration-template) implementation for the [Dr. Fraudsworth](https://github.com/MetalLegBob/drfraudsworth) DeFi protocol on Solana.

## Overview

This crate enables [Titan's Argos routing engine](https://www.titan.com/) to quote and route swaps through Dr. Fraudsworth's on-chain programs. It exposes **6 venue instances** covering **8 swap directions**:

| Venue | Type | Direction | Accounts |
|-------|------|-----------|----------|
| CRIME/SOL Pool | `SolPoolVenue` | SOL <-> CRIME (bidirectional) | 24 buy / 25 sell |
| FRAUD/SOL Pool | `SolPoolVenue` | SOL <-> FRAUD (bidirectional) | 24 buy / 25 sell |
| CRIME -> PROFIT | `VaultVenue` | Unidirectional (100:1 rate) | 17 |
| FRAUD -> PROFIT | `VaultVenue` | Unidirectional (100:1 rate) | 17 |
| PROFIT -> CRIME | `VaultVenue` | Unidirectional (1:100 rate) | 17 |
| PROFIT -> FRAUD | `VaultVenue` | Unidirectional (1:100 rate) | 17 |

## Architecture

```
User swap request
  -> Titan Argos router
    -> TradingVenue::quote()        # Off-chain: pure integer math, < 100us
    -> TradingVenue::generate_swap_instruction()
      -> Tax Program (on-chain)     # Deducts tax, distributes to staking/carnage/treasury
        -> AMM Program (CPI)        # Constant-product swap
          -> Token-2022 (CPI)       # Token transfer
            -> Transfer Hook (CPI)  # Whitelist enforcement
```

Swaps go through the **Tax Program** (not the AMM directly). The Tax Program deducts a dynamic tax based on the current epoch, then CPI-calls the AMM for the actual swap.

### SOL Pool Quote Flow

**Buy (SOL -> token):**
1. Tax deducted from SOL input (epoch-dependent, 3-14%)
2. LP fee deducted (1%)
3. Constant-product swap on post-fee amount

**Sell (token -> SOL):**
1. LP fee deducted from token input (1%)
2. Constant-product swap
3. Tax deducted from SOL output (epoch-dependent, 3-14%)

### Vault Conversions

Fixed 100:1 rate between faction tokens (CRIME/FRAUD) and the yield token (PROFIT). Zero fees, deterministic output.

## Token Details

| Token | Mint | Decimals | Program | Transfer Fee |
|-------|------|----------|---------|-------------|
| SOL | `So111...112` | 9 | SPL Token | None |
| CRIME | `cRiME...PXc` | 6 | Token-2022 | None (uses Transfer Hook) |
| FRAUD | `FraUd...au5` | 6 | Token-2022 | None (uses Transfer Hook) |
| PROFIT | `pRoFi...fR` | 6 | Token-2022 | None (uses Transfer Hook) |

All three protocol tokens use Token-2022 with the **Transfer Hook** extension for whitelist enforcement. They do **not** use the Transfer Fee extension — `TokenInfo.transfer_fee` is `None` for all mints.

## Protocol-Specific Notes

### Minimum Output Floor

The Tax Program enforces `MINIMUM_OUTPUT_FLOOR_BPS = 5000` — the `minimum_output` parameter must be at least 50% of the on-chain computed expected output. Passing `minimum_output = 0` will be rejected.

The adapter sets `minimum_output = quoted_output / 2` by default. Titan may adjust this in their routing wrapper.

### Address Lookup Table

A protocol-wide ALT (`7dy5NNvacB8YkZrc3c96vDMDtacXzxVpdPLiC4B7LJ4h`) is provided via `AddressLookupTableTrait`. The sell path requires 25 accounts, so v0 transactions with ALT compression are recommended.

### Dynamic Tax Rates

Tax rates change every epoch (~30 minutes on mainnet). The adapter reads `EpochState` during `update_state()` to get current rates. Titan should call `update_state()` at least once per epoch for accurate quotes.

## Usage

```rust
use drfraudsworth_titan_adapter::vault_venue::{known_sol_pool_venues, known_vault_venues};

// Get all 6 venue instances
let sol_pools = known_sol_pool_venues();  // 2 venues (uninitialized)
let vaults = known_vault_venues();        // 4 venues (uninitialized)

// Initialize from on-chain state
for venue in &mut sol_pools {
    venue.update_state(&cache).await?;
}
for venue in &mut vaults {
    venue.update_state(&cache).await?;
}

// Quote a swap
let result = sol_pools[0].quote(QuoteRequest {
    input_mint: NATIVE_MINT,
    output_mint: CRIME_MINT,
    amount: 1_000_000_000, // 1 SOL
    swap_type: SwapType::ExactIn,
})?;
println!("Expected output: {} CRIME", result.expected_output);
```

## Test Suite

171 tests covering construction, quoting parity, edge cases, mainnet RPC validation, and speed benchmarks:

```bash
cargo test
```

| Category | Tests | Description |
|----------|-------|-------------|
| Unit tests | 65 | Math, state parsing, account builders, instruction data |
| Construction | 36 | Mock cache, FromAccount, bounds, zero-input, ExactOut |
| Edge gauntlet | 26 | Adversarial inputs (0, 1, u64::MAX, wrong mints) |
| Mainnet validation | 13 | Real mainnet account data, live pool/epoch state |
| Quoting parity | 31 | 300 random samples zero-delta, monotonicity, speed |

## Programs

| Program | Address | Role |
|---------|---------|------|
| Tax Program | `43fZGRtmEsP7ExnJE1dbTbNjaP1ncvVmMPusSeksWGEj` | Swap entry point (deducts tax, CPIs to AMM) |
| AMM | `5JsSAL3kJDUWD4ZveYXYZmgm1eVqueesTZVdAvtZg8cR` | Constant-product swap execution |
| Conversion Vault | `5uawA6ehYTu69Ggvm3LSK84qFawPKxbWgfngwj15NRJ` | Fixed-rate token conversion |
| Transfer Hook | `CiQPQrmQh6BPhb9k7dFnsEs5gKPgdrvNKFc5xie5xVGd` | Whitelist enforcement on T22 transfers |
| Epoch Program | `4Heqc8QEjJCspHR8y96wgZBnBfbe3Qb8N6JBZMQt9iw2` | Tax rate management |
| Staking | `12b3t1cNiAUoYLiWFEnFa4w6qYxVAiqCWU7KZuzLPYtH` | Yield distribution |

## License

MIT

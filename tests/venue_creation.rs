//! Example venue-creation parsing test.
//!
//! Titan tracks new pools live by watching confirmed transactions and detecting
//! when a venue's program creates a new pool. Every integration must provide a
//! `parse_pool_creations` function that turns the decompiled instructions of a
//! transaction into the set of pools it created (see the README requirement and
//! the `YourVenue` stub).
//!
//! This is the worked reference for the Raydium AMM: a self-contained fixture
//! reproducing a real Raydium `initialize2` pool-creation instruction (program
//! id, tag byte, account ordering, and data layout from
//! <https://github.com/raydium-io/raydium-amm/>), parsed with no network
//! access. Unlike the RPC-gated suites, it always runs.

use solana_pubkey::{Pubkey, pubkey};

use titan_integration_template::example::{RAYDIUM_AMM_PROGRAM_ID, parse_pool_creations};
use titan_integration_template::trading_venue::protocol::PoolProtocol;
use titan_integration_template::trading_venue::venue_creation::{ParsedInstruction, PoolCreation};

// Real mainnet addresses, so the fixture is a faithful Raydium pool creation.
const POOL: Pubkey = pubkey!("Bzc9NZfMqkXR6fz1DBph7BDf9BroyEf6pnzESP7v5iiw");
const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
const USDC_MINT: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

/// Build a Raydium `initialize2` instruction in its exact on-chain shape.
///
/// Data is the `InitializeInstruction2` layout prefixed with the tag byte `1`:
/// `[1, nonce: u8, open_time: u64, init_pc_amount: u64, init_coin_amount: u64]`.
/// Accounts follow the program's 21-account ordering; only the new pool (index
/// 4), the coin mint (8) and the pc mint (9) carry information the parser needs.
fn raydium_initialize2(pool: Pubkey, coin_mint: Pubkey, pc_mint: Pubkey) -> ParsedInstruction {
    let mut data = vec![1u8]; // initialize2 discriminator
    data.push(255); // nonce
    data.extend_from_slice(&0u64.to_le_bytes()); // open_time
    data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // init_pc_amount
    data.extend_from_slice(&5_000_000_000u64.to_le_bytes()); // init_coin_amount

    let mut accounts = vec![Pubkey::new_unique(); 21];
    accounts[4] = pool;
    accounts[8] = coin_mint;
    accounts[9] = pc_mint;

    ParsedInstruction {
        program_id: RAYDIUM_AMM_PROGRAM_ID,
        accounts,
        data,
    }
}

/// A Raydium swap (`SwapBaseInV2`, tag `16`) — a real instruction to the AMM
/// program that is *not* a pool creation, so the parser must ignore it.
fn raydium_swap() -> ParsedInstruction {
    let mut data = vec![16u8];
    data.extend_from_slice(&1_000u64.to_le_bytes()); // amount_in
    data.extend_from_slice(&0u64.to_le_bytes()); // minimum_amount_out
    ParsedInstruction {
        program_id: RAYDIUM_AMM_PROGRAM_ID,
        accounts: vec![Pubkey::new_unique(); 8],
        data,
    }
}

#[test]
fn parses_raydium_pool_creation() {
    // A transaction that creates the WSOL/USDC pool, alongside an unrelated swap.
    let instructions = vec![
        raydium_swap(),
        raydium_initialize2(POOL, WSOL_MINT, USDC_MINT),
    ];

    let creations = parse_pool_creations(&instructions);

    assert_eq!(
        creations,
        vec![PoolCreation {
            protocol: PoolProtocol::RaydiumAMM,
            pool: POOL,
            mints: vec![WSOL_MINT, USDC_MINT],
        }],
    );
}

#[test]
fn ignores_transactions_without_a_creation() {
    let creations = parse_pool_creations(&[raydium_swap()]);
    assert!(
        creations.is_empty(),
        "a swap-only transaction creates no pools, got {creations:?}"
    );
}

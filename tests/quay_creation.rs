//! Quay's venue-creation parsing test.
//!
//! Self-contained fixture for Quay's pool-creation instruction (`init_strategy`):
//! no RPC, no network — just the decompiled instruction shape `parse_pool_creations`
//! must recognize. Mirrors `tests/venue_creation.rs`.

use solana_pubkey::{Pubkey, pubkey};

use titan_integration_template::trading_venue::protocol::PoolProtocol;
use titan_integration_template::trading_venue::venue_creation::{ParsedInstruction, PoolCreation};
use titan_integration_template::quay::{QUAY_PROGRAM_ID, parse_pool_creations};

/// `init_strategy` discriminator — the first data byte the parser keys on
/// (`quay_sdk::consts::DISC_INIT_STRATEGY`).
const DISC_INIT_STRATEGY: u8 = 0x10;

// The Strategy account created by `init_strategy` — Quay's "pool".
const POOL: Pubkey = pubkey!("Cs8KY3PiWrCMAytMsBRQo8EdGbticVtdvufLnb2UhXh");
// The new strategy's base mint (here: wrapped SOL).
const TOKEN_A_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
// The new strategy's quote mint (here: USDC).
const TOKEN_B_MINT: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

/// A Quay `init_strategy` instruction. The parser keys on the program id and the
/// leading discriminator byte, then reads the new Strategy at account index 0 and
/// the bound `(base, quote)` mints at positional indices 5 and 6.
fn quay_pool_creation() -> ParsedInstruction {
    let filler = Pubkey::new_unique();
    ParsedInstruction {
        program_id: QUAY_PROGRAM_ID,
        // [strategy, mm, cfg, owner, system, base_mint, quote_mint]
        accounts: vec![
            POOL,
            filler,
            filler,
            filler,
            filler,
            TOKEN_A_MINT,
            TOKEN_B_MINT,
        ],
        data: vec![DISC_INIT_STRATEGY, 0, 0, 0, 0, 0, 0],
    }
}

fn unrelated_instruction() -> ParsedInstruction {
    ParsedInstruction {
        program_id: QUAY_PROGRAM_ID,
        accounts: vec![],
        data: vec![],
    }
}

#[test]
fn parses_quay_pool_creation() {
    let creations = parse_pool_creations(&[quay_pool_creation()]);

    assert_eq!(
        creations,
        vec![PoolCreation {
            protocol: PoolProtocol::Quay,
            pool: POOL,
            mints: vec![TOKEN_A_MINT, TOKEN_B_MINT],
        }],
    );
}

#[test]
fn ignores_transactions_without_a_creation() {
    let creations = parse_pool_creations(&[unrelated_instruction()]);
    assert!(
        creations.is_empty(),
        "a transaction without a pool creation creates no pools, got {creations:?}"
    );
}

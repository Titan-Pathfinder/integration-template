//! Your venue's venue-creation parsing test.
//!
//! Fill this in with a self-contained fixture for your venue's pool-creation
//! instruction. It should mirror `tests/venue_creation.rs`: no RPC, no network,
//! just the decompiled instruction shape that your parser must recognize.

use solana_pubkey::{Pubkey, pubkey};

use titan_integration_template::trading_venue::protocol::PoolProtocol;
use titan_integration_template::trading_venue::venue_creation::{ParsedInstruction, PoolCreation};
use titan_integration_template::your_venue::{YOUR_PROGRAM_ID, parse_pool_creations};

// FILL_IN: replace with a real pool address created by your fixture instruction.
const POOL: Pubkey = pubkey!("11111111111111111111111111111111");
// FILL_IN: replace with the first tradable mint from the new pool.
const TOKEN_A_MINT: Pubkey = pubkey!("11111111111111111111111111111111");
// FILL_IN: replace with the second tradable mint from the new pool.
const TOKEN_B_MINT: Pubkey = pubkey!("11111111111111111111111111111111");

fn require_fixture_constants_replaced() {
    if POOL == Pubkey::default() {
        todo!("replace POOL with a real pool address created by your fixture")
    }
    if TOKEN_A_MINT == Pubkey::default() || TOKEN_B_MINT == Pubkey::default() {
        todo!("replace TOKEN_A_MINT and TOKEN_B_MINT with real tradable mints")
    }
}

fn your_venue_pool_creation() -> ParsedInstruction {
    // FILL_IN: build your venue's real pool-creation instruction fixture.
    // Match the program id, discriminator, account order, and data layout your
    // parser expects. Include the new pool and mint accounts at their real
    // instruction positions.
    todo!("build YourVenue pool-creation instruction fixture")
}

fn unrelated_instruction() -> ParsedInstruction {
    ParsedInstruction {
        program_id: YOUR_PROGRAM_ID,
        accounts: vec![],
        data: vec![],
    }
}

#[test]
fn parses_your_venue_pool_creation() {
    require_fixture_constants_replaced();
    let creations = parse_pool_creations(&[your_venue_pool_creation()]);

    assert_eq!(
        creations,
        vec![PoolCreation {
            protocol: PoolProtocol::YourPoolProtocol,
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

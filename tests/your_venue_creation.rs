//! Pool-creation parsing for the Coffer `Coffer` venue.
//!
//! Self-contained fixture (no RPC) reproducing `initialize_pool` in its
//! on-chain shape on branch `main`: the Anchor discriminator followed by
//! `normalized_weights: Vec<u64>`, `initial_virtual_balances: Vec<u64>`,
//! `swap_fee_rate: u32`, `pool_id: u64`, `banned_extensions_override:
//! Option<u64>`; accounts `config, pool, bpt_mint, payer, token_program,
//! associated_token_program, system_program` followed by one mint per token
//! as `remaining_accounts`. The pool and mints are the real ones of the live
//! 4-token pool `5dDez…` (WSOL, a Token-2022 mint, USDC, USDT).

use solana_pubkey::{Pubkey, pubkey};

use titan_integration_template::coffer_venue::instruction::{
    INITIALIZE_POOL_DISCRIMINATOR, encode_swap_data,
};
use titan_integration_template::coffer_venue::{COFFER_PROGRAM_ID, parse_pool_creations};
use titan_integration_template::trading_venue::protocol::PoolProtocol;
use titan_integration_template::trading_venue::venue_creation::{ParsedInstruction, PoolCreation};

const POOL: Pubkey = pubkey!("5dDezuaofYZUBdab8gSWY3ys86VbMrTxsRuf3ayqe8GJ");
const CONFIG: Pubkey = pubkey!("E9K7CxPXpAEp49Usgxcxxm9DR8qUFXyvpwLjKrqqtrbY");
const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
const T22_MINT: Pubkey = pubkey!("QMemov5tqvCgUt6qXX2Unfb6NrrLzLyZpTWeUdTpJ95");
const USDC_MINT: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const USDT_MINT: Pubkey = pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");

fn borsh_vec_u64(values: &[u64]) -> Vec<u8> {
    let mut out = (values.len() as u32).to_le_bytes().to_vec();
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// `initialize_pool` exactly as a client would encode it.
fn initialize_pool(pool: Pubkey, mints: &[Pubkey]) -> ParsedInstruction {
    let n = mints.len();
    let mut data = INITIALIZE_POOL_DISCRIMINATOR.to_vec();
    data.extend(borsh_vec_u64(&vec![10_000 / n as u64; n])); // normalized_weights
    data.extend(borsh_vec_u64(&vec![1_000_000_000; n])); // initial_virtual_balances
    data.extend_from_slice(&5_000u32.to_le_bytes()); // swap_fee_rate
    data.extend_from_slice(&1_779_829_399_139u64.to_le_bytes()); // pool_id
    data.push(0); // banned_extensions_override = None

    let mut accounts = vec![
        CONFIG,
        pool,
        Pubkey::new_unique(), // bpt_mint PDA
        Pubkey::new_unique(), // payer
        spl_token::ID,
        spl_associated_token_account::ID,
        solana_sdk::system_program::ID,
    ];
    accounts.extend_from_slice(mints);
    ParsedInstruction {
        program_id: COFFER_PROGRAM_ID,
        accounts,
        data,
    }
}

/// A `swap` on the same program: not a creation, must be ignored.
fn coffer_swap() -> ParsedInstruction {
    ParsedInstruction {
        program_id: COFFER_PROGRAM_ID,
        accounts: vec![Pubkey::new_unique(); 10],
        data: encode_swap_data(1_000, 0, 0, 1),
    }
}

fn unrelated_instruction() -> ParsedInstruction {
    ParsedInstruction {
        program_id: COFFER_PROGRAM_ID,
        accounts: vec![],
        data: vec![],
    }
}

#[test]
fn parses_coffer_pool_creation() {
    let mints = [WSOL_MINT, T22_MINT, USDC_MINT, USDT_MINT];
    let instructions = vec![coffer_swap(), initialize_pool(POOL, &mints)];

    let creations = parse_pool_creations(&instructions);

    assert_eq!(
        creations,
        vec![PoolCreation {
            protocol: PoolProtocol::Coffer,
            pool: POOL,
            mints: mints.to_vec(),
        }],
    );
}

#[test]
fn parses_a_two_token_creation_next_to_foreign_instructions() {
    let pool = Pubkey::new_unique();
    let mints = [WSOL_MINT, USDC_MINT];
    // Same discriminator bytes on another program: not ours.
    let foreign = ParsedInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![pool, WSOL_MINT, USDC_MINT],
        data: INITIALIZE_POOL_DISCRIMINATOR.to_vec(),
    };
    let creations = parse_pool_creations(&[foreign, initialize_pool(pool, &mints)]);
    assert_eq!(creations.len(), 1);
    assert_eq!(creations[0].pool, pool);
    assert_eq!(creations[0].mints, mints);
}

#[test]
fn rejects_malformed_creations() {
    // Weight count disagrees with the trailing mint accounts.
    let mut ix = initialize_pool(POOL, &[WSOL_MINT, USDC_MINT]);
    ix.accounts.push(USDT_MINT);
    assert!(parse_pool_creations(&[ix]).is_empty());
    // Too few accounts to hold even the fixed prefix.
    let mut ix = initialize_pool(POOL, &[WSOL_MINT, USDC_MINT]);
    ix.accounts.truncate(3);
    assert!(parse_pool_creations(&[ix]).is_empty());
}

#[test]
fn ignores_transactions_without_a_creation() {
    let creations = parse_pool_creations(&[unrelated_instruction(), coffer_swap()]);
    assert!(
        creations.is_empty(),
        "a transaction without a pool creation creates no pools, got {creations:?}"
    );
}

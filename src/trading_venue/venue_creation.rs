//! Venue-creation parsing contract.
//!
//! Titan tracks new pools live by watching confirmed transactions and detecting
//! when a venue's program creates a new pool. Each integration provides a
//! `parse_pool_creations` function (see the worked Raydium reference in
//! [`crate::example`] and the stub in [`crate::your_venue`]) that maps the
//! decompiled instructions of a transaction to the pools it created.
//!
//! The parser works purely off instruction data, so it stays free of any RPC
//! transaction-encoding types: the caller decompiles a confirmed transaction
//! into [`ParsedInstruction`]s, and the parser pattern-matches them.

use solana_pubkey::Pubkey;

use crate::trading_venue::protocol::PoolProtocol;

/// One instruction from a confirmed transaction, decompiled so that every
/// account reference is resolved to an absolute pubkey.
///
/// Callers are expected to flatten a transaction into these before parsing:
/// resolve the message's account-key indices (including address-lookup-table
/// keys) to pubkeys, and include inner / CPI instructions — some venues create
/// pools via CPI from a router or aggregator, not as a top-level instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedInstruction {
    /// Program the instruction invoked.
    pub program_id: Pubkey,
    /// The instruction's accounts, in order, resolved to absolute pubkeys.
    pub accounts: Vec<Pubkey>,
    /// Raw instruction data (the program's own encoding — discriminator + args).
    pub data: Vec<u8>,
}

/// A new pool discovered by parsing a creation transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolCreation {
    /// Which protocol created the pool.
    pub protocol: PoolProtocol,
    /// The new pool/market account address. Hand this to
    /// [`FromAccount::from_account`](crate::trading_venue::FromAccount::from_account)
    /// to build a venue, exactly as the suite does with a known pool.
    pub pool: Pubkey,
    /// The tradable token mints of the new pool.
    pub mints: Vec<Pubkey>,
}

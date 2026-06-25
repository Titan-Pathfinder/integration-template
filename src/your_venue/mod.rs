//! Skeleton for **your** venue integration.
//!
//! This mirrors the worked reference in [`crate::example`] (a Raydium AMM) but
//! with the venue-specific logic left as `todo!()` for you to fill in. The
//! shared test suite runs against this type via `tests/your_venue.rs`, so as you
//! implement the `FILL_IN` methods below those tests go from red to green — the
//! same bar the example already meets.
//!
//! Recommended order: `parse_pool_creations` → `from_account` → `update_state`
//! → `get_token_info` → `quote` (output **and** the marginal price) →
//! `generate_swap_instruction`.

use ahash::HashSet;
use async_trait::async_trait;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use crate::{
    account_caching::AccountsCache,
    trading_venue::{
        FromAccount, QuoteRequest, QuoteResult, TradingVenue,
        error::TradingVenueError,
        protocol::PoolProtocol,
        token_info::TokenInfo,
        venue_creation::{ParsedInstruction, PoolCreation},
    },
};

// FILL_IN: replace with your venue's on-chain program id.
pub const YOUR_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

fn require_your_program_id_replaced() {
    if YOUR_PROGRAM_ID == Pubkey::default() {
        todo!("replace YOUR_PROGRAM_ID with your venue's on-chain program id")
    }
}

/// Detect every pool your venue created in a confirmed transaction.
///
/// Titan tracks new pools live by feeding the decompiled instructions of
/// confirmed transactions through this function; each returned
/// [`PoolCreation::pool`] is then built into a venue via
/// [`YourVenue::from_account`]. See [`crate::trading_venue::venue_creation`] for
/// the contract and `tests/venue_creation.rs` for the worked Raydium reference.
pub fn parse_pool_creations(instructions: &[ParsedInstruction]) -> Vec<PoolCreation> {
    // FILL_IN: scan `instructions` for the call(s) that create one of your pools
    // (match `program_id` to your program and the data's discriminator to your
    // pool-creation instruction), then read the new pool account and its token
    // mints out of that instruction's accounts. Mirror `example::parse_pool_creations`.
    let _ = instructions;
    todo!("YourVenue::parse_pool_creations — detect your venue's pool-creation instruction")
}

/// Your venue's off-chain state. Add whatever the quote math needs.
#[derive(Clone)]
pub struct YourVenue {
    /// Address of the pool/market account this venue was built from.
    pub pool_id: Pubkey,
    /// Token metadata (mints + decimals + token programs), populated in
    /// `update_state`.
    token_info: Vec<TokenInfo>,
    /// Accounts that must be fetched to refresh quoting state.
    required_state_pubkeys: HashSet<Pubkey>,
    /// Set to `true` once all required state has been loaded.
    initialized: bool,
    // FILL_IN: add your pool's quoting state here (reserves, fees, tick arrays,
    // an invariant config, an orderbook snapshot, ...).
}

#[allow(dead_code)]
fn fill_in_your_venue_state_fields() -> ! {
    todo!("add your pool's quoting state fields to YourVenue")
}

impl FromAccount for YourVenue {
    fn from_account(pubkey: &Pubkey, account: &Account) -> Result<Self, TradingVenueError> {
        // FILL_IN: deserialize your pool account into the struct above and record
        // the pubkeys you'll need to fetch in `update_state` (vaults, mints, ...).
        let _ = (pubkey, account);
        todo!("YourVenue::from_account — parse your pool account into venue state")
    }
}

#[async_trait]
impl TradingVenue for YourVenue {
    fn initialized(&self) -> bool {
        self.initialized
    }

    fn program_id(&self) -> Pubkey {
        require_your_program_id_replaced();
        YOUR_PROGRAM_ID
    }

    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![self.program_id()]
    }

    fn market_id(&self) -> Pubkey {
        self.pool_id
    }

    fn get_token_info(&self) -> &[TokenInfo] {
        &self.token_info
    }

    fn protocol(&self) -> PoolProtocol {
        // FILL_IN: rename `YourPoolProtocol` in trading_venue::protocol to your
        // protocol, then return it here.
        todo!("rename YourPoolProtocol and return your real PoolProtocol variant")
    }

    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(self.required_state_pubkeys.iter().cloned().collect())
    }

    async fn update_state(&mut self, cache: &dyn AccountsCache) -> Result<(), TradingVenueError> {
        // FILL_IN: fetch your required accounts through `cache`, deserialize the
        // live state, populate `token_info`, and set `initialized = true`.
        let _ = cache;
        todo!("YourVenue::update_state — load live pool state from the cache")
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
        // FILL_IN: compute the output for `request.amount` and the marginal price
        // `f'(amount)` in raw output atoms per raw input atom. See
        // `QuoteResult::price` and `crate::example::price` for the
        // constant-product derivation.
        let _ = request;
        todo!("YourVenue::quote — return expected_output and the marginal price")
    }

    fn generate_swap_instruction(
        &self,
        request: QuoteRequest,
        user: Pubkey,
    ) -> Result<Instruction, TradingVenueError> {
        // FILL_IN: build your on-chain program's swap instruction for `user`.
        let _ = (request, user);
        todo!("YourVenue::generate_swap_instruction — build your swap instruction")
    }
}

//! Core traits and data structures used by Titan-compatible trading venues.
//!
//! A "trading venue" is any automated market maker (AMM), orderbook, or
//! proprietary liquidity engine that wishes to integrate with Titan’s quoting
//! and routing framework.
//!
//! Implementers are responsible for correctly handling state updates,
//! account deserialization, quoting semantics, and swap instruction
//! generation. This template captures the information Titan needs to add
//! support for a venue.

pub mod bounds;
pub mod error;
pub mod protocol;
pub mod token_info;
pub mod venue_creation;

use async_trait::async_trait;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use crate::{
    account_caching::AccountsCache,
    trading_venue::{
        bounds::find_boundaries, error::TradingVenueError, protocol::PoolProtocol,
        token_info::TokenInfo,
    },
};

/// Describes which type of swap the user is performing.
///
/// **Titan only supports `ExactIn`.** It only ever calls a venue with
/// `ExactIn`, and `ExactOut` is not routed today. Venues are not required to
/// implement `ExactOut` — returning an error for it is fine — but must not
/// panic. The variant is kept as a forward-compat signal only.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SwapType {
    /// The user specifies exactly how many input atoms they want to spend, and
    /// the venue returns a quote for the resulting output amount. The only mode
    /// Titan supports today.
    ExactIn,
    /// The user specifies exactly how many output atoms they want to receive,
    /// and the venue determines how many input atoms are required. Reserved for
    /// future use — Titan does NOT route `ExactOut` today.
    ExactOut,
}

/// Request structure passed to venue `quote()` and `generate_swap_instruction()`.
///
/// All amounts are denominated in integer atom units (not scaled to UI decimals).
#[derive(Debug, Clone)]
pub struct QuoteRequest {
    /// Mint of the token the user is providing.
    pub input_mint: Pubkey,

    /// Mint of the token the user expects to receive.
    pub output_mint: Pubkey,

    /// Amount of *input* or *output* atoms, depending on `swap_type`.
    pub amount: u64,

    /// Swap mode. Titan only ever sets this to `ExactIn`; venues need not
    /// implement `ExactOut` and may return an error for it, but must not panic.
    pub swap_type: SwapType,
}

/// A result returned from a venue’s `quote()` implementation.
///
/// This describes how much of the input would be consumed and how much
/// output would be produced, based on current pool state.
#[derive(Debug, Clone)]
pub struct QuoteResult {
    /// Mint of the token the user provided.
    pub input_mint: Pubkey,

    /// Mint of the token the user receives.
    pub output_mint: Pubkey,

    /// Actual amount of input atoms that would be consumed.
    pub amount: u64,

    /// Expected number of output atoms produced by the venue.
    pub expected_output: u64,

    /// Indicates whether the pool has insufficient liquidity to consume the full input.
    ///
    /// For example, if a pool only has enough liquidity for half of the provided
    /// input, this flag should be set to `true` and `amount = request.amount / 2`.
    pub not_enough_liquidity: bool,

    /// Price at the requested amount.
    ///
    /// Let `f(x)` be the number of output atoms produced for an `ExactIn` swap
    /// of `x` input atoms against the current pool state. `price` is the
    /// instantaneous exchange rate at the quoted size — the derivative of the
    /// output curve with respect to the input, evaluated at `amount`:
    ///
    /// ```text
    /// price = f'(amount)        // output atoms per input atom
    /// ```
    ///
    /// `price` is the marginal derivative of the raw quote curve:
    ///
    /// ```text
    /// price = d(output_atoms) / d(input_atoms)
    /// ```
    ///
    /// Do not apply UI decimal scaling here. Use the same raw atom units as
    /// `request.amount` and `expected_output`. At `amount == 0` this is the
    /// venue's *spot price*.
    ///
    /// How you obtain the derivative is up to you. Titan does not prescribe a method, but
    /// the value **must** satisfy the invariants described on [`TradingVenue::quote`]:
    /// it must be positive on a valid quote, non-increasing as `amount` grows
    /// (concavity), and consistent with the realized output (the mean value
    /// theorem). These properties are exercised by the pricing test suite. You **must** provide
    /// a spot price at 0.
    pub price: f64,
}

/// A convenience trait for converting on-chain accounts into structured pool/venue state.
///
/// Implementers are responsible for performing any deserialization necessary
/// to reconstruct on-chain pool state for their venue.
pub trait FromAccount {
    /// Parse an on-chain Solana account into the venue’s internal state structure.
    ///
    /// `pubkey` is the address of the account; `account` is its data.
    fn from_account(pubkey: &Pubkey, account: &Account) -> Result<Self, TradingVenueError>
    where
        Self: Sized;
}

/// Trait allowing a venue to declare which address-lookup table (ALT) keys
/// it requires for transaction construction.
///
/// Implementers should return all additional keys (besides swap accounts)
/// that must be included in the ALT in order to successfully compress swaps.
#[async_trait]
pub trait AddressLookupTableTrait {
    /// Return a list of pubkeys that should be inserted into an address lookup table.
    async fn get_lookup_table_keys(
        &self,
        accounts_cache: Option<&dyn AccountsCache>,
    ) -> Result<Vec<Pubkey>, TradingVenueError>;
}

/// Public template trait describing an AMM or trading venue for Titan integration.
///
/// Any AMM, orderbook, or custom liquidity engine must implement this trait
/// to be usable by Titan’s routing system.
#[async_trait]
pub trait TradingVenue {
    /// Whether the venue is fully initialized.
    ///
    /// This allows Titan to skip venues that failed initialization or
    /// are missing required on-chain accounts.
    fn initialized(&self) -> bool;

    /// The main program ID for the venue.
    fn program_id(&self) -> Pubkey;

    /// All additional program IDs this venue depends on (e.g. SPL Token program).
    fn program_dependencies(&self) -> Vec<Pubkey>;

    /// Unique identifier for the market/pool instance.
    fn market_id(&self) -> Pubkey;

    /// Return the mint pubkeys for all tokens traded in this venue.
    ///
    /// The default implementation pulls these from the venue's `TokenInfo`.
    fn tradable_mints(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(self.get_token_info().iter().map(|x| x.pubkey).collect())
    }

    /// Return every declared input/output token-index pair this venue can quote.
    ///
    /// The default assumes every distinct pair in `get_token_info()` is tradable.
    /// Override this if your pool has more than two tokens but only supports a
    /// subset of directions.
    fn directions_num(&self) -> Vec<(u8, u8)> {
        let indices: Vec<u8> = (0..self.get_token_info().len())
            .filter_map(|index| u8::try_from(index).ok())
            .collect();

        indices
            .iter()
            .flat_map(|&i| {
                indices
                    .iter()
                    .filter(move |&&j| j != i)
                    .map(move |&j| (i, j))
            })
            .collect()
    }

    /// Return the decimals for each tradable token.
    fn decimals(&self) -> Result<Vec<i32>, TradingVenueError> {
        Ok(self.get_token_info().iter().map(|x| x.decimals).collect())
    }

    /// Return fixed token metadata for this venue (mint + decimals).
    fn get_token_info(&self) -> &[TokenInfo];

    /// Fetch a single token by index.
    ///
    /// Returns an error if the index is out of bounds.
    fn get_token(&self, i: usize) -> Result<&TokenInfo, TradingVenueError> {
        self.get_token_info()
            .get(i)
            .ok_or(TradingVenueError::TokenInfoIndexError(i))
    }

    /// Identify which protocol type this venue is (e.g. Raydium, Orca, Phoenix).
    fn protocol(&self) -> PoolProtocol;

    /// A human-readable label describing the venue’s protocol.
    fn label(&self) -> String {
        self.protocol().into()
    }

    /// Returns the minimal set of pubkeys required to update venue state.
    ///
    /// Titan will prefetch these accounts before calling `update_state()`.
    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError>;

    /// Update the venue's internal state from the provided account cache.
    ///
    /// This is where implementers deserialize pool accounts, tick arrays,
    /// orderbooks, or other relevant on-chain state.
    async fn update_state(&mut self, cache: &dyn AccountsCache) -> Result<(), TradingVenueError>;

    /// Compute a quote for the given swap parameters.
    ///
    /// **Implementer requirement:** the venue **must** handle zero input amounts
    /// without panicking or returning an error. Titan sometimes requests zero-input
    /// quotes.
    ///
    /// Titan only ever calls this with `SwapType::ExactIn`. Venues need not
    /// implement `ExactOut` (returning an error for it is acceptable) but must
    /// not panic on it.
    ///
    /// # Pricing requirements
    ///
    /// In addition to `expected_output`, every quote must report a price
    /// (see [`QuoteResult::price`]). Titan relies on this price for
    /// routing, so the quote function `f(x) -> output` and the price
    /// `p(x) = f'(x)` it reports must be mutually consistent and well-behaved on
    /// the venue's valid input range `[lower_bound, upper_bound]`:
    ///
    /// 1. **Monotonic output.** `f` is non-decreasing: a larger `ExactIn` amount
    ///    never returns less output.
    ///
    /// 2. **Monotonic (non-increasing) price / concavity.** `p` is non-increasing
    ///    in `amount`. Larger fills receive a weaker rate; the output
    ///    curve is concave. In particular `price > 0` for any valid quote.
    ///
    /// 3. **Mean value theorem.** The reported price must bracket the realized
    ///    average rate over any interval. For `a < b`, the chord
    ///
    ///    ```text
    ///    chord = (f(b) - f(a)) / (b - a)
    ///    ```
    ///
    ///    must satisfy `p(b) <= chord <= p(a)`. Equivalently, there is some
    ///    `c in [a, b]` with `p(c) == chord`: the price you quote is the genuine
    ///    derivative of the output you quote, not an unrelated number.
    ///
    /// These invariants are checked directly by the pricing tests shipped with
    /// this template (monotonicity and mean-value-theorem tests). A venue whose
    /// `price` is inconsistent with its `expected_output` will fail them.
    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError>;

    /// Construct the transaction instruction needed to execute a swap.
    ///
    /// This should use the amounts from the original `QuoteRequest`,
    /// not the `QuoteResult`. Venues should not modify swap semantics here;
    /// only build the appropriate on-chain instruction.
    fn generate_swap_instruction(
        &self,
        request: QuoteRequest,
        user: Pubkey,
    ) -> Result<Instruction, TradingVenueError>;

    /// Compute lower/upper admissible boundaries for valid input amounts
    /// using binary search over the venue's `quote()` function.
    ///
    /// This is used by Titan when determining safe routing ranges or when
    /// generating fallback limits.
    ///
    /// `tkn_in_ind` and `tkn_out_ind` refer to token indices in `get_token_info()`.
    fn bounds(&self, tkn_in_ind: u8, tkn_out_ind: u8) -> Result<(u64, u64), TradingVenueError> {
        let input_mint = self.get_token(tkn_in_ind as usize)?.pubkey;
        let output_mint = self.get_token(tkn_out_ind as usize)?.pubkey;

        // Closure for boundary-finding—performs `ExactIn` quotes at various x.
        let f = |x: u64| {
            self.quote(QuoteRequest {
                amount: x,
                swap_type: SwapType::ExactIn,
                input_mint,
                output_mint,
            })
        };

        find_boundaries(&f)
    }
}

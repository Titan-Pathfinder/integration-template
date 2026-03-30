// Titan TradingVenue implementation for Dr. Fraudsworth SOL pool swaps.
//
// SolPoolVenue handles CRIME/SOL and FRAUD/SOL swaps via the Tax Program.
// Two instances are created: one per pool.
//
// Quote flow (identical to Jupiter adapter):
//   Buy (SOL -> token): tax deducted from SOL INPUT, then LP fee, then AMM swap
//   Sell (token -> SOL): LP fee, then AMM swap, then tax deducted from SOL OUTPUT

use async_trait::async_trait;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use crate::account_caching::AccountsCache;
use crate::trading_venue::error::{ErrorInfo, TradingVenueError};
use crate::trading_venue::protocol::PoolProtocol;
use crate::trading_venue::token_info::TokenInfo;
use crate::trading_venue::{
    AddressLookupTableTrait, FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
};

use crate::drfraudsworth::accounts::addresses::{
    CRIME_MINT, CRIME_SOL_POOL, EPOCH_STATE_PDA, FRAUD_MINT, FRAUD_SOL_POOL,
    NATIVE_MINT, PROTOCOL_ALT, TAX_PROGRAM_ID, AMM_PROGRAM_ID, TRANSFER_HOOK_PROGRAM_ID,
    TOKEN_2022_PROGRAM_ID,
};
use crate::drfraudsworth::accounts::sol_pool_accounts::{build_buy_account_metas, build_sell_account_metas};
use crate::drfraudsworth::instruction_data::{build_swap_buy_data, build_swap_sell_data};
use crate::drfraudsworth::math::amm_math::{calculate_effective_input, calculate_swap_output};
use crate::drfraudsworth::math::tax_math::calculate_tax;
use crate::drfraudsworth::state::epoch_state::ParsedEpochState;
use crate::drfraudsworth::state::pool_state::ParsedPoolState;
use crate::drfraudsworth::token_info_builder::token_info_for_mint;

/// Titan TradingVenue for CRIME/SOL and FRAUD/SOL pools.
#[derive(Clone)]
pub struct SolPoolVenue {
    pool_address: Pubkey,
    is_crime: bool,
    reserve_sol: u64,
    reserve_token: u64,
    lp_fee_bps: u16,
    buy_tax_bps: u16,
    sell_tax_bps: u16,
    token_info: Vec<TokenInfo>,
    initialized: bool,
}

impl SolPoolVenue {
    /// Create a venue with known values (for testing).
    pub fn new_for_testing(
        is_crime: bool,
        reserve_sol: u64,
        reserve_token: u64,
        buy_tax_bps: u16,
        sell_tax_bps: u16,
    ) -> Self {
        let pool_address = if is_crime { CRIME_SOL_POOL } else { FRAUD_SOL_POOL };
        let token_mint = if is_crime { CRIME_MINT } else { FRAUD_MINT };

        Self {
            pool_address,
            is_crime,
            reserve_sol,
            reserve_token,
            lp_fee_bps: crate::drfraudsworth::constants::LP_FEE_BPS,
            buy_tax_bps,
            sell_tax_bps,
            token_info: vec![
                token_info_for_mint(&NATIVE_MINT).unwrap(),
                token_info_for_mint(&token_mint).unwrap(),
            ],
            initialized: true,
        }
    }

    /// Create an uninitialized venue (for factory use — call update_state before quoting).
    pub fn new_uninitialized(is_crime: bool, pool_address: Pubkey, token_mint: Pubkey) -> Self {
        Self {
            pool_address,
            is_crime,
            reserve_sol: 0,
            reserve_token: 0,
            lp_fee_bps: crate::drfraudsworth::constants::LP_FEE_BPS,
            buy_tax_bps: 0,
            sell_tax_bps: 0,
            token_info: vec![
                token_info_for_mint(&NATIVE_MINT).unwrap(),
                token_info_for_mint(&token_mint).unwrap(),
            ],
            initialized: false,
        }
    }

    fn token_mint(&self) -> Pubkey {
        if self.is_crime { CRIME_MINT } else { FRAUD_MINT }
    }

    /// Quote a buy (SOL -> token).
    #[allow(clippy::result_large_err)] // TradingVenueError is Titan's type, can't change its size
    fn quote_buy(&self, amount_in: u64) -> Result<QuoteResult, TradingVenueError> {
        let token_mint = self.token_mint();

        if amount_in == 0 {
            return Ok(QuoteResult {
                input_mint: NATIVE_MINT,
                output_mint: token_mint,
                amount: 0,
                expected_output: 0,
                not_enough_liquidity: false,
            });
        }

        // 1. Tax deducted from SOL input
        let tax = calculate_tax(amount_in, self.buy_tax_bps)
            .ok_or_else(|| TradingVenueError::CheckedMathError(
                ErrorInfo::StaticStr("Tax calculation overflow in buy"),
            ))?;

        let sol_to_swap = amount_in.checked_sub(tax)
            .ok_or_else(|| TradingVenueError::CheckedMathError(
                ErrorInfo::StaticStr("Tax exceeds input amount"),
            ))?;

        if sol_to_swap == 0 {
            return Ok(QuoteResult {
                input_mint: NATIVE_MINT,
                output_mint: token_mint,
                amount: amount_in,
                expected_output: 0,
                not_enough_liquidity: false,
            });
        }

        // 2. LP fee deducted from post-tax amount
        let effective_input = calculate_effective_input(sol_to_swap, self.lp_fee_bps)
            .ok_or_else(|| TradingVenueError::CheckedMathError(
                ErrorInfo::StaticStr("LP fee calculation overflow"),
            ))?;

        // 3. Constant-product swap
        let out_amount = calculate_swap_output(
            self.reserve_sol,
            self.reserve_token,
            effective_input,
        )
        .ok_or_else(|| TradingVenueError::CheckedMathError(
            ErrorInfo::StaticStr("Swap output calculation overflow"),
        ))?;

        // Constant-product output asymptotically approaches but never reaches reserves.
        // Flag when output exceeds 95% of reserves — signals Titan this venue can't
        // meaningfully fill at this size.
        let not_enough_liquidity = out_amount > self.reserve_token * 95 / 100;

        Ok(QuoteResult {
            input_mint: NATIVE_MINT,
            output_mint: token_mint,
            amount: amount_in,
            expected_output: out_amount,
            not_enough_liquidity,
        })
    }

    /// Quote a sell (token -> SOL).
    #[allow(clippy::result_large_err)]
    fn quote_sell(&self, amount_in: u64) -> Result<QuoteResult, TradingVenueError> {
        let token_mint = self.token_mint();

        if amount_in == 0 {
            return Ok(QuoteResult {
                input_mint: token_mint,
                output_mint: NATIVE_MINT,
                amount: 0,
                expected_output: 0,
                not_enough_liquidity: false,
            });
        }

        // 1. LP fee deducted from token input
        let effective_input = calculate_effective_input(amount_in, self.lp_fee_bps)
            .ok_or_else(|| TradingVenueError::CheckedMathError(
                ErrorInfo::StaticStr("LP fee calculation overflow"),
            ))?;

        // 2. Constant-product swap (token -> SOL)
        let gross_sol = calculate_swap_output(
            self.reserve_token,
            self.reserve_sol,
            effective_input,
        )
        .ok_or_else(|| TradingVenueError::CheckedMathError(
            ErrorInfo::StaticStr("Swap output calculation overflow"),
        ))?;

        // 3. Tax deducted from SOL output
        let tax = calculate_tax(gross_sol, self.sell_tax_bps)
            .ok_or_else(|| TradingVenueError::CheckedMathError(
                ErrorInfo::StaticStr("Tax calculation overflow in sell"),
            ))?;

        let net_sol = gross_sol.checked_sub(tax)
            .ok_or_else(|| TradingVenueError::CheckedMathError(
                ErrorInfo::StaticStr("Tax exceeds gross output"),
            ))?;

        let not_enough_liquidity = gross_sol > self.reserve_sol * 95 / 100;

        Ok(QuoteResult {
            input_mint: token_mint,
            output_mint: NATIVE_MINT,
            amount: amount_in,
            expected_output: net_sol,
            not_enough_liquidity,
        })
    }
}

impl FromAccount for SolPoolVenue {
    fn from_account(
        pubkey: &Pubkey,
        account: &solana_sdk::account::Account,
    ) -> Result<Self, TradingVenueError> {
        let pool_state = ParsedPoolState::from_bytes(&account.data)
            .map_err(|e| TradingVenueError::DeserializationFailed(
                ErrorInfo::String(format!("PoolState: {}", e)),
            ))?;

        let (reserve_sol, reserve_token) = pool_state.sol_and_token_reserves();
        let is_crime = *pubkey == CRIME_SOL_POOL;
        let token_mint = if is_crime { CRIME_MINT } else { FRAUD_MINT };

        Ok(Self {
            pool_address: *pubkey,
            is_crime,
            reserve_sol,
            reserve_token,
            lp_fee_bps: pool_state.lp_fee_bps,
            buy_tax_bps: 0,
            sell_tax_bps: 0,
            token_info: vec![
                token_info_for_mint(&NATIVE_MINT).unwrap(),
                token_info_for_mint(&token_mint).unwrap(),
            ],
            initialized: false, // Not initialized until update_state() loads epoch tax rates
        })
    }
}

#[async_trait]
impl TradingVenue for SolPoolVenue {
    fn initialized(&self) -> bool {
        self.initialized
    }

    fn program_id(&self) -> Pubkey {
        TAX_PROGRAM_ID
    }

    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![AMM_PROGRAM_ID, TRANSFER_HOOK_PROGRAM_ID, TOKEN_2022_PROGRAM_ID]
    }

    fn market_id(&self) -> Pubkey {
        self.pool_address
    }

    fn tradable_mints(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(vec![NATIVE_MINT, self.token_mint()])
    }

    fn decimals(&self) -> Result<Vec<i32>, TradingVenueError> {
        Ok(self.token_info.iter().map(|t| t.decimals).collect())
    }

    fn get_token_info(&self) -> &[TokenInfo] {
        &self.token_info
    }

    fn get_token(&self, i: usize) -> Result<&TokenInfo, TradingVenueError> {
        self.token_info.get(i).ok_or(TradingVenueError::TokenInfoIndexError(i))
    }

    fn protocol(&self) -> PoolProtocol {
        // TODO: Replace with PoolProtocol::DrFraudsworth when we fork the template
        PoolProtocol::DrFraudsworth
    }

    fn label(&self) -> String {
        if self.is_crime {
            "Dr Fraudsworth CRIME/SOL".to_string()
        } else {
            "Dr Fraudsworth FRAUD/SOL".to_string()
        }
    }

    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(vec![self.pool_address, EPOCH_STATE_PDA])
    }

    async fn update_state(
        &mut self,
        cache: &dyn AccountsCache,
    ) -> Result<(), TradingVenueError> {
        // Fetch pool + epoch accounts
        let accounts = cache.get_accounts(&[self.pool_address, EPOCH_STATE_PDA]).await?;

        let pool_account = accounts[0].as_ref()
            .ok_or_else(|| TradingVenueError::NoAccountFound(
                ErrorInfo::Pubkey(self.pool_address),
            ))?;

        let epoch_account = accounts[1].as_ref()
            .ok_or_else(|| TradingVenueError::NoAccountFound(
                ErrorInfo::Pubkey(EPOCH_STATE_PDA),
            ))?;

        // Parse pool state
        let pool_state = ParsedPoolState::from_bytes(&pool_account.data)
            .map_err(|e| TradingVenueError::DeserializationFailed(
                ErrorInfo::String(format!("PoolState: {}", e)),
            ))?;
        let (reserve_sol, reserve_token) = pool_state.sol_and_token_reserves();
        self.reserve_sol = reserve_sol;
        self.reserve_token = reserve_token;
        self.lp_fee_bps = pool_state.lp_fee_bps;

        // Parse epoch state for tax rates
        let epoch_state = ParsedEpochState::from_bytes(&epoch_account.data)
            .map_err(|e| TradingVenueError::DeserializationFailed(
                ErrorInfo::String(format!("EpochState: {}", e)),
            ))?;
        self.buy_tax_bps = epoch_state.get_tax_bps(self.is_crime, true);
        self.sell_tax_bps = epoch_state.get_tax_bps(self.is_crime, false);

        self.initialized = true;
        Ok(())
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
        if request.swap_type == SwapType::ExactOut {
            return Err(TradingVenueError::ExactOutNotSupported);
        }

        let is_buy = request.input_mint == NATIVE_MINT;

        if is_buy {
            self.quote_buy(request.amount)
        } else {
            self.quote_sell(request.amount)
        }
    }

    fn generate_swap_instruction(
        &self,
        request: QuoteRequest,
        user: Pubkey,
    ) -> Result<Instruction, TradingVenueError> {
        let is_buy = request.input_mint == NATIVE_MINT;
        let token_mint = self.token_mint();

        // Derive user ATAs
        let user_wsol_ata = spl_associated_token_account::get_associated_token_address(
            &user, &NATIVE_MINT,
        );
        let user_token_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            &user, &token_mint, &crate::drfraudsworth::accounts::addresses::TOKEN_2022_PROGRAM_ID,
        );

        let (account_metas, data) = if is_buy {
            let metas = build_buy_account_metas(&user, &user_wsol_ata, &user_token_ata, self.is_crime);
            // NOTE: Our Tax Program enforces a 50% output floor (MINIMUM_OUTPUT_FLOOR_BPS=5000).
            // min_amount_out=0 will be rejected on-chain. Titan must set this to at least
            // 50% of expected_output. For now we pass the quoted output as min (tight slippage).
            // Titan may adjust this in their wrapper based on their slippage strategy.
            let quoted = self.quote_buy(request.amount)
                .map(|r| r.expected_output)
                .unwrap_or(0);
            let data = build_swap_buy_data(request.amount, quoted / 2, self.is_crime);
            (metas, data)
        } else {
            let metas = build_sell_account_metas(&user, &user_token_ata, &user_wsol_ata, self.is_crime);
            let quoted = self.quote_sell(request.amount)
                .map(|r| r.expected_output)
                .unwrap_or(0);
            let data = build_swap_sell_data(request.amount, quoted / 2, self.is_crime);
            (metas, data)
        };

        Ok(Instruction {
            program_id: TAX_PROGRAM_ID,
            accounts: account_metas,
            data,
        })
    }

    fn bounds(
        &self,
        tkn_in_ind: u8,
        tkn_out_ind: u8,
    ) -> Result<(u64, u64), TradingVenueError> {
        // Use Titan's default boundary search via binary search over quote()
        let input_mint = self.token_info.get(tkn_in_ind as usize)
            .ok_or(TradingVenueError::TokenInfoIndexError(tkn_in_ind as usize))?.pubkey;
        let output_mint = self.token_info.get(tkn_out_ind as usize)
            .ok_or(TradingVenueError::TokenInfoIndexError(tkn_out_ind as usize))?.pubkey;

        let f = |amount: u64| -> Result<QuoteResult, TradingVenueError> {
            self.quote(QuoteRequest {
                input_mint,
                output_mint,
                amount,
                swap_type: SwapType::ExactIn,
            })
        };

        crate::trading_venue::bounds::find_boundaries(&f)
    }
}

#[async_trait]
impl AddressLookupTableTrait for SolPoolVenue {
    async fn get_lookup_table_keys(
        &self,
        _accounts_cache: Option<&dyn AccountsCache>,
    ) -> Result<Vec<Pubkey>, TradingVenueError> {
        // Our protocol uses a single pre-built ALT containing all program IDs,
        // PDAs, pool addresses, mints, and vaults. This enables v0 transactions
        // for the sell path which has 25 accounts.
        Ok(vec![PROTOCOL_ALT])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_venue(is_crime: bool) -> SolPoolVenue {
        SolPoolVenue::new_for_testing(is_crime, 100_000_000_000, 100_000_000_000, 400, 1400)
    }

    #[test]
    fn buy_quote_applies_tax_then_swap() {
        let venue = make_venue(true);
        let result = venue.quote(QuoteRequest {
            input_mint: NATIVE_MINT,
            output_mint: CRIME_MINT,
            amount: 1_000_000_000,
            swap_type: SwapType::ExactIn,
        }).unwrap();

        assert!(result.expected_output > 0);
        assert!(!result.not_enough_liquidity);
    }

    #[test]
    fn sell_quote_applies_tax_after_swap() {
        let venue = make_venue(true);
        let result = venue.quote(QuoteRequest {
            input_mint: CRIME_MINT,
            output_mint: NATIVE_MINT,
            amount: 1_000_000_000,
            swap_type: SwapType::ExactIn,
        }).unwrap();

        assert!(result.expected_output > 0);
    }

    #[test]
    fn zero_amount_returns_zero_output() {
        let venue = make_venue(true);
        let result = venue.quote(QuoteRequest {
            input_mint: NATIVE_MINT,
            output_mint: CRIME_MINT,
            amount: 0,
            swap_type: SwapType::ExactIn,
        }).unwrap();

        assert_eq!(result.expected_output, 0);
    }

    #[test]
    fn exact_out_returns_error() {
        let venue = make_venue(true);
        let result = venue.quote(QuoteRequest {
            input_mint: NATIVE_MINT,
            output_mint: CRIME_MINT,
            amount: 1_000_000_000,
            swap_type: SwapType::ExactOut,
        });

        assert!(result.is_err());
    }

    #[test]
    fn tradable_mints_correct() {
        let crime_venue = make_venue(true);
        let mints = crime_venue.tradable_mints().unwrap();
        assert_eq!(mints, vec![NATIVE_MINT, CRIME_MINT]);

        let fraud_venue = make_venue(false);
        let mints = fraud_venue.tradable_mints().unwrap();
        assert_eq!(mints, vec![NATIVE_MINT, FRAUD_MINT]);
    }

    #[test]
    fn token_info_has_two_entries() {
        let venue = make_venue(true);
        assert_eq!(venue.get_token_info().len(), 2);
        assert_eq!(venue.get_token_info()[0].pubkey, NATIVE_MINT);
        assert_eq!(venue.get_token_info()[1].pubkey, CRIME_MINT);
    }

    #[test]
    fn label_includes_pool_name() {
        assert_eq!(make_venue(true).label(), "Dr Fraudsworth CRIME/SOL");
        assert_eq!(make_venue(false).label(), "Dr Fraudsworth FRAUD/SOL");
    }

    #[test]
    fn market_id_is_pool_address() {
        let venue = make_venue(true);
        assert_eq!(venue.market_id(), CRIME_SOL_POOL);
    }
}

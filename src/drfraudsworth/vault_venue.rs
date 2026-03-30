// Titan TradingVenue implementation for Dr. Fraudsworth Conversion Vault.
//
// VaultVenue handles 4 conversion directions at fixed 100:1 rate:
//   CRIME -> PROFIT (divide by 100)
//   FRAUD -> PROFIT (divide by 100)
//   PROFIT -> CRIME (multiply by 100)
//   PROFIT -> FRAUD (multiply by 100)
//
// Each instance is unidirectional. Zero fees. Deterministic output.

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
    CONVERSION_VAULT_PROGRAM_ID, CRIME_MINT, CRIME_SOL_POOL, FRAUD_MINT, FRAUD_SOL_POOL,
    PROFIT_MINT, PROTOCOL_ALT, TOKEN_2022_PROGRAM_ID, TRANSFER_HOOK_PROGRAM_ID, VAULT_CONFIG_PDA,
};
use crate::drfraudsworth::sol_pool_venue::SolPoolVenue;
use crate::drfraudsworth::accounts::vault_accounts::build_vault_account_metas;
use crate::drfraudsworth::instruction_data::build_convert_data;
use crate::drfraudsworth::math::vault_math::compute_vault_output;
use crate::drfraudsworth::token_info_builder::token_info_for_mint;

/// Titan TradingVenue for Conversion Vault (fixed-rate token conversions).
///
/// Each instance represents one unidirectional conversion.
#[derive(Clone)]
pub struct VaultVenue {
    input_mint: Pubkey,
    output_mint: Pubkey,
    token_info: Vec<TokenInfo>,
    initialized: bool,
}

impl VaultVenue {
    pub fn new_for_testing(input_mint: Pubkey, output_mint: Pubkey) -> Self {
        Self {
            input_mint,
            output_mint,
            token_info: vec![
                token_info_for_mint(&input_mint).unwrap(),
                token_info_for_mint(&output_mint).unwrap(),
            ],
            initialized: true,
        }
    }
}

impl FromAccount for VaultVenue {
    fn from_account(
        _pubkey: &Pubkey,
        _account: &solana_sdk::account::Account,
    ) -> Result<Self, TradingVenueError> {
        // Vaults are discovered via known_vault_venues(), not from on-chain accounts.
        // This is provided for trait completeness but shouldn't be the primary path.
        Err(TradingVenueError::UnsupportedVenue(
            ErrorInfo::StaticStr("Use known_vault_venues() to construct VaultVenue instances"),
        ))
    }
}

#[async_trait]
impl TradingVenue for VaultVenue {
    fn initialized(&self) -> bool {
        self.initialized
    }

    fn program_id(&self) -> Pubkey {
        CONVERSION_VAULT_PROGRAM_ID
    }

    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![TRANSFER_HOOK_PROGRAM_ID, TOKEN_2022_PROGRAM_ID]
    }

    fn market_id(&self) -> Pubkey {
        // Synthetic key derived from mint pair (same as Jupiter adapter)
        let (key, _) = Pubkey::find_program_address(
            &[b"titan_vault", self.input_mint.as_ref(), self.output_mint.as_ref()],
            &CONVERSION_VAULT_PROGRAM_ID,
        );
        key
    }

    fn tradable_mints(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(vec![self.input_mint, self.output_mint])
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
        format!("Dr Fraudsworth {} Vault", format_pair(&self.input_mint, &self.output_mint))
    }

    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(vec![VAULT_CONFIG_PDA])
    }

    async fn update_state(
        &mut self,
        cache: &dyn AccountsCache,
    ) -> Result<(), TradingVenueError> {
        // Just verify VaultConfig exists. Rates are fixed at 100:1.
        let account = cache.get_account(&VAULT_CONFIG_PDA).await?;
        let account = account.ok_or_else(|| TradingVenueError::NoAccountFound(
            ErrorInfo::Pubkey(VAULT_CONFIG_PDA),
        ))?;
        if account.data.is_empty() {
            return Err(TradingVenueError::MissingState(
                ErrorInfo::StaticStr("VaultConfig account has no data"),
            ));
        }

        self.initialized = true;
        Ok(())
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
        if request.swap_type == SwapType::ExactOut {
            return Err(TradingVenueError::ExactOutNotSupported);
        }

        // Handle zero amount gracefully (Titan requirement)
        if request.amount == 0 {
            return Ok(QuoteResult {
                input_mint: self.input_mint,
                output_mint: self.output_mint,
                amount: 0,
                expected_output: 0,
                not_enough_liquidity: false,
            });
        }

        // Verify direction matches
        if request.input_mint != self.input_mint {
            return Err(TradingVenueError::InvalidMint(
                ErrorInfo::Pubkey(request.input_mint),
            ));
        }

        let out_amount = compute_vault_output(
            &self.input_mint,
            &self.output_mint,
            request.amount,
        )
        .ok_or_else(|| TradingVenueError::CheckedMathError(
            ErrorInfo::StaticStr("Vault conversion failed (dust too small, overflow, or invalid pair)"),
        ))?;

        Ok(QuoteResult {
            input_mint: self.input_mint,
            output_mint: self.output_mint,
            amount: request.amount,
            expected_output: out_amount,
            not_enough_liquidity: false, // Vaults have protocol-funded reserves
        })
    }

    fn generate_swap_instruction(
        &self,
        request: QuoteRequest,
        user: Pubkey,
    ) -> Result<Instruction, TradingVenueError> {
        // Derive user ATAs (both T22)
        let user_input_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            &user, &self.input_mint, &TOKEN_2022_PROGRAM_ID,
        );
        let user_output_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            &user, &self.output_mint, &TOKEN_2022_PROGRAM_ID,
        );

        let account_metas = build_vault_account_metas(
            &user,
            &user_input_ata,
            &user_output_ata,
            &self.input_mint,
            &self.output_mint,
        );

        let data = build_convert_data(request.amount);

        Ok(Instruction {
            program_id: CONVERSION_VAULT_PROGRAM_ID,
            accounts: account_metas,
            data,
        })
    }

    fn bounds(
        &self,
        tkn_in_ind: u8,
        tkn_out_ind: u8,
    ) -> Result<(u64, u64), TradingVenueError> {
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
impl AddressLookupTableTrait for VaultVenue {
    async fn get_lookup_table_keys(
        &self,
        _accounts_cache: Option<&dyn AccountsCache>,
    ) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(vec![PROTOCOL_ALT])
    }
}

// =============================================================================
// Factory functions
// =============================================================================

/// Returns 2 pre-built SolPoolVenue instances (uninitialized — call update_state).
pub fn known_sol_pool_venues() -> Vec<SolPoolVenue> {
    let token_mints = [(true, CRIME_MINT, CRIME_SOL_POOL), (false, FRAUD_MINT, FRAUD_SOL_POOL)];

    token_mints
        .iter()
        .map(|(is_crime, token_mint, pool)| {
            SolPoolVenue::new_uninitialized(*is_crime, *pool, *token_mint)
        })
        .collect()
}

/// Returns 4 pre-built VaultVenue instances (one per direction).
pub fn known_vault_venues() -> Vec<VaultVenue> {
    let pairs = [
        (CRIME_MINT, PROFIT_MINT),
        (FRAUD_MINT, PROFIT_MINT),
        (PROFIT_MINT, CRIME_MINT),
        (PROFIT_MINT, FRAUD_MINT),
    ];

    pairs
        .iter()
        .map(|(input, output)| VaultVenue {
            input_mint: *input,
            output_mint: *output,
            token_info: vec![
                token_info_for_mint(input).unwrap(),
                token_info_for_mint(output).unwrap(),
            ],
            initialized: false,
        })
        .collect()
}

/// Returns all 6 venues (2 SOL pools + 4 vaults).
pub fn all_venues() -> (Vec<SolPoolVenue>, Vec<VaultVenue>) {
    (known_sol_pool_venues(), known_vault_venues())
}

// =============================================================================
// Helpers
// =============================================================================

fn mint_name(mint: &Pubkey) -> &'static str {
    if *mint == CRIME_MINT { "CRIME" }
    else if *mint == FRAUD_MINT { "FRAUD" }
    else if *mint == PROFIT_MINT { "PROFIT" }
    else { "UNKNOWN" }
}

fn format_pair(input: &Pubkey, output: &Pubkey) -> String {
    format!("{}/{}", mint_name(input), mint_name(output))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crime_to_profit_quote() {
        let venue = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
        let result = venue.quote(QuoteRequest {
            input_mint: CRIME_MINT,
            output_mint: PROFIT_MINT,
            amount: 10_000,
            swap_type: SwapType::ExactIn,
        }).unwrap();

        assert_eq!(result.expected_output, 100);
        assert!(!result.not_enough_liquidity);
    }

    #[test]
    fn profit_to_fraud_quote() {
        let venue = VaultVenue::new_for_testing(PROFIT_MINT, FRAUD_MINT);
        let result = venue.quote(QuoteRequest {
            input_mint: PROFIT_MINT,
            output_mint: FRAUD_MINT,
            amount: 50,
            swap_type: SwapType::ExactIn,
        }).unwrap();

        assert_eq!(result.expected_output, 5_000);
    }

    #[test]
    fn zero_amount_returns_zero() {
        let venue = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
        let result = venue.quote(QuoteRequest {
            input_mint: CRIME_MINT,
            output_mint: PROFIT_MINT,
            amount: 0,
            swap_type: SwapType::ExactIn,
        }).unwrap();

        assert_eq!(result.expected_output, 0);
    }

    #[test]
    fn exact_out_returns_error() {
        let venue = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
        let result = venue.quote(QuoteRequest {
            input_mint: CRIME_MINT,
            output_mint: PROFIT_MINT,
            amount: 10_000,
            swap_type: SwapType::ExactOut,
        });
        assert!(result.is_err());
    }

    #[test]
    fn dust_too_small_errors() {
        let venue = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
        let result = venue.quote(QuoteRequest {
            input_mint: CRIME_MINT,
            output_mint: PROFIT_MINT,
            amount: 99, // 99 / 100 = 0
            swap_type: SwapType::ExactIn,
        });
        assert!(result.is_err());
    }

    #[test]
    fn known_vault_venues_returns_4() {
        let venues = known_vault_venues();
        assert_eq!(venues.len(), 4);
    }

    #[test]
    fn known_sol_pool_venues_returns_2() {
        let venues = known_sol_pool_venues();
        assert_eq!(venues.len(), 2);
    }

    #[test]
    fn all_venues_returns_6_total() {
        let (sol_pools, vaults) = all_venues();
        assert_eq!(sol_pools.len() + vaults.len(), 6);
    }

    #[test]
    fn vault_label_includes_pair() {
        let venue = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
        assert_eq!(venue.label(), "Dr Fraudsworth CRIME/PROFIT Vault");
    }

    #[test]
    fn vault_token_info_has_two_entries() {
        let venue = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
        assert_eq!(venue.get_token_info().len(), 2);
        assert_eq!(venue.get_token_info()[0].pubkey, CRIME_MINT);
        assert_eq!(venue.get_token_info()[1].pubkey, PROFIT_MINT);
    }
}

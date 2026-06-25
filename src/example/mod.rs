use ahash::HashSet;
use async_trait::async_trait;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address;

use crate::{
    account_caching::AccountsCache,
    example::{
        amm::{
            AmmKeys, CalculateResult, calculate_pool_vault_amounts_from_accounts, load_amm_keys,
            swap_v2, swap_with_slippage,
        },
        raydium::math::SwapDirection,
    },
    trading_venue::{
        AddressLookupTableTrait, FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
        error::TradingVenueError,
        protocol::PoolProtocol,
        token_info::TokenInfo,
        venue_creation::{ParsedInstruction, PoolCreation},
    },
};

mod amm;
mod raydium;

pub const RAYDIUM_AMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

/// Detect every Raydium AMM pool created by a confirmed transaction.
///
/// This is the live-pool-tracking half of the integration: Titan feeds the
/// decompiled instructions of confirmed transactions through here, and each
/// returned [`PoolCreation::pool`] is then built into a venue via
/// [`RaydiumAmmVenue::from_account`]. See
/// [`crate::trading_venue::venue_creation`] for the contract and
/// `tests/venue_creation.rs` for a worked fixture.
pub fn parse_pool_creations(instructions: &[ParsedInstruction]) -> Vec<PoolCreation> {
    // Raydium creates a pool with `initialize2` (tag byte 1). The new pool, coin
    // mint and pc mint sit at fixed positions in its account list:
    // https://github.com/raydium-io/raydium-amm/ (program/src/instruction.rs).
    const INITIALIZE2_TAG: u8 = 1;
    const POOL_INDEX: usize = 4;
    const COIN_MINT_INDEX: usize = 8;
    const PC_MINT_INDEX: usize = 9;

    instructions
        .iter()
        .filter(|ix| ix.program_id == RAYDIUM_AMM_PROGRAM_ID)
        .filter(|ix| ix.data.first() == Some(&INITIALIZE2_TAG))
        .filter_map(|ix| {
            // Defensive: a real `initialize2` always carries these accounts, but
            // never index past a malformed instruction.
            let pool = *ix.accounts.get(POOL_INDEX)?;
            let coin_mint = *ix.accounts.get(COIN_MINT_INDEX)?;
            let pc_mint = *ix.accounts.get(PC_MINT_INDEX)?;
            Some(PoolCreation {
                protocol: PoolProtocol::RaydiumAMM,
                pool,
                mints: vec![coin_mint, pc_mint],
            })
        })
        .collect()
}

/// Price (`f'(amount)`) of the constant-product curve, in output
/// atoms per input atom. See [`crate::trading_venue::QuoteResult::price`].
///
/// This pool is a constant product `x * y = k` with a proportional input fee.
/// Writing `X` for the input reserve, `Y` for the output reserve and `phi` for
/// the fee fraction, the output for an `ExactIn` swap of `x` input atoms is
///
/// ```text
/// x_eff = (1 - phi) * x
/// f(x)  = Y * x_eff / (X + x_eff)
/// ```
///
/// which we differentiate by hand to get the price
///
/// ```text
/// f'(x) = Y * (1 - phi) * X / (X + x_eff)^2
/// ```
///
/// This is positive, decreasing in `x` (the curve is concave) and equals the
/// true derivative of `f`, so it satisfies the monotonicity and mean-value
/// invariants Titan's pricing tests check for.
fn price(calculate_result: &CalculateResult, swap_direction: SwapDirection, amount: u64) -> f64 {
    // Reserves are oriented so that `input_reserve` always backs the token
    // being sold and `output_reserve` the token being bought.
    let (input_reserve, output_reserve) = match swap_direction {
        // Selling coin, receiving pc.
        SwapDirection::Coin2PC => (
            calculate_result.pool_coin_vault_amount as f64,
            calculate_result.pool_pc_vault_amount as f64,
        ),
        // Selling pc, receiving coin.
        SwapDirection::PC2Coin => (
            calculate_result.pool_pc_vault_amount as f64,
            calculate_result.pool_coin_vault_amount as f64,
        ),
    };

    let fee_fraction = if calculate_result.swap_fee_denominator == 0 {
        0.0
    } else {
        calculate_result.swap_fee_numerator as f64 / calculate_result.swap_fee_denominator as f64
    };
    let keep = 1.0 - fee_fraction;

    let effective_in = keep * amount as f64;
    let denom = input_reserve + effective_in;

    output_reserve * keep * input_reserve / (denom * denom)
}

#[derive(Clone)]
pub struct RaydiumAmmVenue {
    pub amm_keys: AmmKeys,
    pub calculate_result: Option<CalculateResult>,
    pub pc_balance: u64,
    pub coin_balance: u64,
    required_state_pubkeys: HashSet<Pubkey>,
    found_all_pubkeys: bool,
    token_info: Vec<TokenInfo>,
}

impl FromAccount for RaydiumAmmVenue {
    fn from_account(pubkey: &Pubkey, account: &Account) -> Result<Self, TradingVenueError> {
        let amm_keys = load_amm_keys(&RAYDIUM_AMM_PROGRAM_ID, pubkey, account)
            .map_err(|_| TradingVenueError::FromAccountError("Unable to unpack AmmInfo".into()))?;

        // Just missing market's event_q
        let required_state_pubkeys = HashSet::from_iter([
            amm_keys.amm_pool,
            amm_keys.amm_pc_vault,
            amm_keys.amm_coin_vault,
            amm_keys.amm_coin_mint,
            amm_keys.amm_pc_mint,
        ]);
        let found_all_pubkeys = false;

        Ok(Self {
            amm_keys,
            calculate_result: None,
            pc_balance: 0,
            coin_balance: 0,
            required_state_pubkeys,
            found_all_pubkeys,
            token_info: Vec::new(),
        })
    }
}

#[async_trait]
impl TradingVenue for RaydiumAmmVenue {
    fn initialized(&self) -> bool {
        self.found_all_pubkeys
    }

    fn market_id(&self) -> Pubkey {
        self.amm_keys.amm_pool
    }

    fn program_id(&self) -> Pubkey {
        RAYDIUM_AMM_PROGRAM_ID
    }

    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![RAYDIUM_AMM_PROGRAM_ID]
    }
    fn protocol(&self) -> PoolProtocol {
        PoolProtocol::RaydiumAMM
    }

    fn tradable_mints(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(vec![self.amm_keys.amm_coin_mint, self.amm_keys.amm_pc_mint])
    }

    fn decimals(&self) -> Result<Vec<i32>, TradingVenueError> {
        Ok(vec![
            self.token_info
                .first()
                .ok_or_else(|| TradingVenueError::MissingState(self.amm_keys.amm_coin_mint.into()))?
                .decimals,
            self.token_info
                .get(1)
                .ok_or_else(|| TradingVenueError::MissingState(self.amm_keys.amm_pc_mint.into()))?
                .decimals,
        ])
    }

    fn get_token_info(&self) -> &[TokenInfo] {
        &self.token_info
    }

    async fn update_state(&mut self, cache: &dyn AccountsCache) -> Result<(), TradingVenueError> {
        let accounts_pubkeys = vec![
            self.amm_keys.amm_pool,
            self.amm_keys.amm_pc_vault,
            self.amm_keys.amm_coin_vault,
            self.amm_keys.amm_coin_mint,
            self.amm_keys.amm_pc_mint,
        ];

        self.required_state_pubkeys.extend(&accounts_pubkeys);
        //
        let accounts = cache.get_accounts(&accounts_pubkeys).await?;

        let [
            amm_pool_account,
            amm_pc_vault_account,
            amm_coin_vault_account,
            mint0_account,
            mint1_account,
        ]: [Option<Account>; 5] = accounts
            .try_into()
            .map_err(|_| TradingVenueError::FailedToFetchMultipleAccountData)?;

        self.venue_update_state_step_2(
            amm_pool_account.clone(),
            amm_pc_vault_account.clone(),
            amm_coin_vault_account.clone(),
            mint0_account.clone(),
            mint1_account.clone(),
        )?;

        self.found_all_pubkeys = true;

        Ok(())
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
        // TODO: Create an error for this to throw.
        let calculate_result = self
            .calculate_result
            .ok_or(TradingVenueError::MissingState("calculate_result".into()))?;

        let swap_direction = if request.input_mint.eq(&self.amm_keys.amm_pc_mint)
            && request.output_mint.eq(&self.amm_keys.amm_coin_mint)
        {
            SwapDirection::PC2Coin
        } else if request.input_mint.eq(&self.amm_keys.amm_coin_mint)
            && request.output_mint.eq(&self.amm_keys.amm_pc_mint)
        {
            SwapDirection::Coin2PC
        } else {
            return Err(TradingVenueError::InvalidMint(request.input_mint.into()));
        };

        // A zero-input quote produces no output, but Titan still expects the
        // venue's spot price. Short-circuit here: `swap_exact_amount` rejects a
        // zero fee and would otherwise error on `amount == 0`.
        if request.amount == 0 {
            return Ok(QuoteResult {
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                amount: 0,
                expected_output: 0,
                not_enough_liquidity: false,
                price: price(&calculate_result, swap_direction, 0),
            });
        }

        let output_amount = swap_with_slippage(
            self.pc_balance,
            self.coin_balance,
            calculate_result.pool_pc_vault_amount,
            calculate_result.pool_coin_vault_amount,
            calculate_result.swap_fee_numerator,
            calculate_result.swap_fee_denominator,
            swap_direction,
            request.amount,
            request.swap_type == SwapType::ExactIn,
            0,
        )?;

        let price = price(&calculate_result, swap_direction, request.amount);

        Ok(QuoteResult {
            input_mint: request.input_mint,
            output_mint: request.output_mint,
            amount: request.amount,
            expected_output: output_amount,
            not_enough_liquidity: false,
            price,
        })
    }

    fn generate_swap_instruction(
        &self,
        request: QuoteRequest,
        user: Pubkey,
    ) -> Result<Instruction, TradingVenueError> {
        let user_token_mint_coin =
            get_associated_token_address(&user, &self.amm_keys.amm_coin_mint);
        let user_token_mint_pc = get_associated_token_address(&user, &self.amm_keys.amm_pc_mint);

        let (user_source, user_destination) = if request.input_mint.eq(&self.amm_keys.amm_coin_mint)
        {
            (user_token_mint_coin, user_token_mint_pc)
        } else {
            (user_token_mint_pc, user_token_mint_coin)
        };

        let ix = swap_v2(
            &self.program_id(),
            &self.amm_keys,
            &user,
            &user_source,
            &user_destination,
            request.amount,
            0,
            true,
        )
        .map_err(|_| TradingVenueError::AmmMethodError("generate swap instruction".into()))?;

        Ok(ix)
    }

    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        if !self.found_all_pubkeys {
            return Err(TradingVenueError::NotInitialized(
                "State needs to be fully updated!".into(),
            ));
        }
        Ok(self
            .required_state_pubkeys
            .iter()
            .cloned()
            .collect::<Vec<Pubkey>>())
    }
}

impl RaydiumAmmVenue {
    #[allow(clippy::too_many_arguments)]
    fn venue_update_state_step_2(
        &mut self,
        amm_pool: Option<Account>,
        amm_pc_vault: Option<Account>,
        amm_coin_vault: Option<Account>,
        mint0: Option<Account>,
        mint1: Option<Account>,
    ) -> Result<(), TradingVenueError> {
        if let Some(vault) = amm_pc_vault.as_ref() {
            let data: [u8; 8] = match vault.data[64..72].try_into() {
                Ok(val) => val,
                Err(_) => {
                    return Err(TradingVenueError::DeserializationFailed(
                        "Failed to deserialize bytes for vault balance".into(),
                    ));
                }
            };
            self.pc_balance = u64::from_le_bytes(data);
        }

        if let Some(vault) = amm_coin_vault.as_ref() {
            let data: [u8; 8] = match vault.data[64..72].try_into() {
                Ok(val) => val,
                Err(_) => {
                    return Err(TradingVenueError::DeserializationFailed(
                        "Failed to deserialize bytes for vault balance".into(),
                    ));
                }
            };
            self.coin_balance = u64::from_le_bytes(data);
        }

        self.calculate_result = Some(
            calculate_pool_vault_amounts_from_accounts(&[amm_pool, amm_pc_vault, amm_coin_vault])
                .map_err(|_| {
                TradingVenueError::AmmMethodError("calculate_pool_vault_amounts".into())
            })?,
        );

        if let [Some(token_mint_0), Some(token_mint_1)] = [mint0, mint1] {
            self.token_info = vec![
                TokenInfo::new(&self.amm_keys.amm_coin_mint, &token_mint_0, u64::MAX)?,
                TokenInfo::new(&self.amm_keys.amm_pc_mint, &token_mint_1, u64::MAX)?,
            ];
        }
        Ok(())
    }
}

#[async_trait]
impl AddressLookupTableTrait for RaydiumAmmVenue {
    async fn get_lookup_table_keys(
        &self,
        accounts_cache: Option<&dyn AccountsCache>,
    ) -> Result<Vec<Pubkey>, TradingVenueError> {
        let amm_pc_mint = self.amm_keys.amm_pc_mint;
        let amm_coin_mint = self.amm_keys.amm_coin_mint;

        let pool_id = self.amm_keys.amm_pool;
        let amm_coin_vault = self.amm_keys.amm_coin_vault;
        let amm_pc_vault = self.amm_keys.amm_pc_vault;

        let rpc_cache = accounts_cache
            .ok_or_else(|| TradingVenueError::SomethingWentWrong("RPC cache required".into()))?;

        let token_mint_accounts = rpc_cache
            .get_accounts(&[amm_coin_mint, amm_pc_mint])
            .await?;

        let coin_mint_account =
            token_mint_accounts[0]
                .as_ref()
                .ok_or(TradingVenueError::MissingState(
                    "token_mint_accounts[0]".into(),
                ))?;
        let pc_mint_account =
            token_mint_accounts[1]
                .as_ref()
                .ok_or(TradingVenueError::MissingState(
                    "token_mint_accounts[1]".into(),
                ))?;

        let coin_mint_program = coin_mint_account.owner;
        let pc_mint_program = pc_mint_account.owner;

        let result_vec = vec![
            amm_coin_mint,
            amm_pc_mint,
            coin_mint_program,
            pc_mint_program,
            pool_id,
            amm_coin_vault,
            amm_pc_vault,
            RAYDIUM_AMM_PROGRAM_ID,
        ];

        Ok(result_vec)
    }
}

#[cfg(test)]
mod price_math_tests {
    use super::{CalculateResult, price};
    use crate::example::amm::swap_exact_amount;
    use crate::example::raydium::math::SwapDirection;

    fn sample_pool() -> CalculateResult {
        CalculateResult {
            // Roughly a SOL/USDC-shaped pool, in atom units.
            pool_pc_vault_amount: 5_000_000_000_000,
            pool_coin_vault_amount: 30_000_000_000_000,
            pool_lp_amount: 0,
            swap_fee_numerator: 25,
            swap_fee_denominator: 10_000,
        }
    }

    /// The analytic price must agree with a central finite difference
    /// of the venue's actual (integer) output curve. This is the property the
    /// mean-value-theorem test relies on at runtime.
    #[test]
    fn price_matches_finite_difference() {
        let cr = sample_pool();

        for dir in [SwapDirection::Coin2PC, SwapDirection::PC2Coin] {
            for &x in &[1_000_000u64, 50_000_000, 1_000_000_000, 50_000_000_000] {
                let analytic = price(&cr, dir, x);

                // Central difference of the real swap math, fee included.
                let h = (x / 1000).max(1_000);
                let f = |amt: u64| {
                    swap_exact_amount(
                        cr.pool_pc_vault_amount,
                        cr.pool_coin_vault_amount,
                        cr.swap_fee_numerator,
                        cr.swap_fee_denominator,
                        dir,
                        amt,
                        true,
                    )
                    .unwrap() as f64
                };
                let fd = (f(x + h) - f(x - h)) / (2.0 * h as f64);

                let rel = (analytic - fd).abs() / fd.abs().max(1e-9);
                assert!(
                    rel < 1e-2,
                    "dir={dir:?} x={x}: analytic={analytic} finite_diff={fd} rel_err={rel}"
                );
            }
        }
    }

    /// The price is positive and strictly decreasing in size
    /// (the curve is concave), matching the monotonicity test's expectation.
    #[test]
    fn price_is_positive_and_decreasing() {
        let cr = sample_pool();

        for dir in [SwapDirection::Coin2PC, SwapDirection::PC2Coin] {
            let mut prev = f64::INFINITY;
            for &x in &[0u64, 1_000, 1_000_000, 1_000_000_000, 100_000_000_000] {
                let p = price(&cr, dir, x);
                assert!(p > 0.0, "dir={dir:?} x={x}: price {p} not positive");
                assert!(
                    p <= prev,
                    "dir={dir:?} x={x}: price {p} increased above {prev}"
                );
                prev = p;
            }
        }
    }
}

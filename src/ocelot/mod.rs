//! Ocelot's native issuer venue. Reference execution uses captured program binaries, never a wallet.
//! The simulation quoter is a correctness oracle, not a sub-microsecond production quoter.
pub mod codec;
pub mod instruction;
pub mod simulation;

use crate::{
    account_caching::AccountsCache,
    trading_venue::{
        error::TradingVenueError,
        protocol::PoolProtocol,
        token_info::TokenInfo,
        venue_creation::{ParsedInstruction, PoolCreation},
        *,
    },
};
use async_trait::async_trait;
use codec::{Config, Mark, Market, Position, Result, invalid, key, u64_at};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use solana_sysvar::clock::Clock;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub const PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("CpLah6JAC9967YiVQnQJegfCKEjLEcUYmx3PCYgg9xWf");
pub const USDC: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
pub const PHOENIX: Pubkey = Pubkey::from_str_const("EtrnLzgbS7nMMy5fbD42kXiUzGg8XQzJ972Xtk1cjWih");
pub const PHOENIX_GLOBAL: Pubkey =
    Pubkey::from_str_const("2zskx2iyCvb6Stg7RBZkt1f6MrF4dpYtMG3yMvKwqtUZ");
pub const PHOENIX_LOG: Pubkey =
    Pubkey::from_str_const("GdxfTLSsdSY37G6fZoYtdGDSfgFnbT2EmRpuePZxWShS");
pub const PERP_MAP: Pubkey = Pubkey::from_str_const("2nHGAaEw3D5dd4hVueaUNoygkQFmoeKqRQWnSPqSMFUC");
pub const COLLATERAL: Pubkey =
    Pubkey::from_str_const("PhUsd11YkbjSaWjFncfAAmatntsjx3MgDR9B6g1ks3A");
pub const EMBER: Pubkey = Pubkey::from_str_const("EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy");
pub fn discriminator(name: &str) -> [u8; 8] {
    let hash = Sha256::digest(name.as_bytes());
    let mut out = [0; 8];
    out.copy_from_slice(&hash[..8]);
    out
}
pub fn global_bucket() -> Pubkey {
    Pubkey::find_program_address(&[b"withdraw_throttle", b"global_v1"], &PROGRAM_ID).0
}

pub fn parse_pool_creations(instructions: &[ParsedInstruction]) -> Vec<PoolCreation> {
    let disc = discriminator("global:init_market");
    let mut seen = HashSet::new();
    instructions
        .iter()
        .filter_map(|ix| {
            if ix.program_id != PROGRAM_ID
                || ix.data.len() != 31
                || ix.data.get(..8) != Some(&disc)
                || ix.accounts.len() < 10
            {
                return None;
            }
            let id = codec::u16_at(&ix.data, 8).ok()?;
            if ix.data[11] != 2
                || !(1..=5).contains(&ix.data[12])
                || ix.data[13] > 1
                || ix.data[14] != 0
                || ix.accounts[1] != USDC
                || ix.accounts[0]
                    != Pubkey::find_program_address(&[b"market", &id.to_le_bytes()], &PROGRAM_ID).0
                || ix.accounts[2]
                    != Pubkey::find_program_address(&[b"lt2", &id.to_le_bytes()], &PROGRAM_ID).0
                || !seen.insert(ix.accounts[0])
            {
                return None;
            }
            Some(PoolCreation {
                protocol: PoolProtocol::Ocelot,
                pool: ix.accounts[0],
                mints: vec![USDC, ix.accounts[2]],
            })
        })
        .collect()
}

#[derive(Clone)]
pub struct State {
    pub market: Market,
    pub config: Config,
    pub mark: Mark,
    pub position: Position,
    pub clock: Clock,
    pub actual_supply: u64,
    pub revision: u64,
    pub gross: u64,
    pub liability: u64,
    pub exchange: instruction::Exchange,
    pub accounts: HashMap<Pubkey, Account>,
}
#[derive(Clone)]
pub struct OcelotVenue {
    pub pool_id: Pubkey,
    mints: Vec<TokenInfo>,
    state: Option<Arc<State>>,
    simulator: Option<Arc<simulation::ExecutionOracle>>,
}
impl FromAccount for OcelotVenue {
    fn from_account(pubkey: &Pubkey, account: &Account) -> Result<Self> {
        let m = Market::decode(*pubkey, account)?;
        Ok(Self {
            pool_id: *pubkey,
            mints: vec![
                TokenInfo {
                    pubkey: m.usdc,
                    decimals: 6,
                    ..Default::default()
                },
                TokenInfo {
                    pubkey: m.mint,
                    decimals: 6,
                    ..Default::default()
                },
            ],
            state: None,
            simulator: None,
        })
    }
}
async fn required(cache: &dyn AccountsCache, key: Pubkey) -> Result<Account> {
    cache
        .get_account(&key)
        .await?
        .ok_or(TradingVenueError::NoAccountFound(key.into()))
}
impl OcelotVenue {
    pub fn direction(&self, r: &QuoteRequest) -> Result<bool> {
        if !matches!(r.swap_type, SwapType::ExactIn) {
            return Err(TradingVenueError::ExactOutNotSupported);
        }
        if r.input_mint == self.mints[0].pubkey && r.output_mint == self.mints[1].pubkey {
            Ok(true)
        } else if r.input_mint == self.mints[1].pubkey && r.output_mint == self.mints[0].pubkey {
            Ok(false)
        } else {
            Err(TradingVenueError::InvalidMint(r.input_mint.into()))
        }
    }
    pub fn state(&self) -> Result<&State> {
        self.state
            .as_deref()
            .ok_or(TradingVenueError::NotInitialized("Ocelot".into()))
    }
    pub fn execution_oracle(&self) -> Result<&simulation::ExecutionOracle> {
        self.simulator
            .as_deref()
            .ok_or(TradingVenueError::NotInitialized("execution oracle".into()))
    }
    pub fn reference_quote(&self, r: &QuoteRequest) -> Result<simulation::ExecutionResult> {
        let mint = self.direction(r)?;
        let s = self.state()?;
        if s.market.paused || (mint && s.market.deposits_paused) {
            return Err(TradingVenueError::InactivePoolError(
                self.pool_id,
                PoolProtocol::Ocelot,
            ));
        }
        if r.amount == 0 {
            return Ok(simulation::ExecutionResult {
                output: 0,
                compute_units: 0,
                packet_bytes: 0,
                input_remaining: 0,
            });
        }
        let ix = instruction::build(self, r.amount, mint, simulation::USER, 1)?;
        self.simulator
            .as_ref()
            .ok_or(TradingVenueError::NotInitialized("execution oracle".into()))?
            .execute(ix, r.input_mint, r.output_mint, r.amount)
    }
    pub fn build_with_minimum(
        &self,
        r: &QuoteRequest,
        user: Pubkey,
        min_output: u64,
    ) -> Result<Instruction> {
        instruction::build(self, r.amount, self.direction(r)?, user, min_output)
    }
}
#[async_trait]
impl TradingVenue for OcelotVenue {
    fn initialized(&self) -> bool {
        self.state.is_some() && self.simulator.is_some()
    }
    fn program_id(&self) -> Pubkey {
        PROGRAM_ID
    }
    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![
            PROGRAM_ID,
            PHOENIX,
            EMBER,
            spl_token::ID,
            spl_associated_token_account::ID,
        ]
    }
    fn market_id(&self) -> Pubkey {
        self.pool_id
    }
    fn get_token_info(&self) -> &[TokenInfo] {
        &self.mints
    }
    fn protocol(&self) -> PoolProtocol {
        PoolProtocol::Ocelot
    }
    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>> {
        let mut keys = vec![
            self.pool_id,
            instruction::pda(b"phoenix", self.pool_id),
            instruction::pda(b"finance_v1", self.pool_id),
            instruction::pda(b"market_cash_v1", self.pool_id),
            PHOENIX_GLOBAL,
            PERP_MAP,
            solana_sdk_ids::sysvar::clock::ID,
            solana_sdk_ids::sysvar::last_restart_slot::ID,
            global_bucket(),
        ];
        keys.extend(self.program_dependencies());
        if let Some(oracle) = &self.simulator {
            keys.extend(oracle.programdata_keys.iter().copied());
        }
        keys.extend(self.mints.iter().map(|t| t.pubkey));
        if let Some(s) = &self.state {
            keys.extend(s.accounts.keys().copied());
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }
    async fn update_state(&mut self, cache: &dyn AccountsCache) -> Result<()> {
        // A failed refresh invalidates old quotes. Never retain a usable stale simulator.
        let keys = self.get_required_pubkeys_for_update()?;
        self.state = None;
        self.simulator = None;
        let values = cache.get_accounts(&keys).await?;
        if values.len() != keys.len() {
            return Err(invalid("cache returned wrong account count"));
        }
        let mut accounts: HashMap<_, _> = keys
            .into_iter()
            .zip(values)
            .filter_map(|(k, v)| v.map(|a| (k, a)))
            .collect();
        let read = |k: &Pubkey| {
            accounts
                .get(k)
                .ok_or(TradingVenueError::NoAccountFound((*k).into()))
        };
        let m = Market::decode(self.pool_id, read(&self.pool_id)?)?;
        let c = Config::decode(
            self.pool_id,
            read(&instruction::pda(b"phoenix", self.pool_id))?,
        )?;
        let clock: Clock = bincode::deserialize(&read(&solana_sdk_ids::sysvar::clock::ID)?.data)
            .map_err(|_| invalid("invalid clock"))?;
        let mark = codec::mark(read(&PERP_MAP)?, &c, clock.slot)?;
        let f = codec::account(
            read(&instruction::pda(b"finance_v1", self.pool_id))?,
            PROGRAM_ID,
            &discriminator("account:MarketFinanceV1"),
            59,
        )?;
        if f[8] != 1 || key(f, 9)? != self.pool_id || !codec::boolean(f, 41)? {
            return Err(invalid("inactive/mismatched finance"));
        }
        let revision = u64_at(f, 43)?;
        let cash = u64_at(f, 51)?;
        let g = codec::account(
            read(&PHOENIX_GLOBAL)?,
            PHOENIX,
            &discriminator("account:global_configuration"),
            1104,
        )?;
        if key(g, 296)? != COLLATERAL || key(g, 360)? != PERP_MAP {
            return Err(invalid("Phoenix global identity mismatch"));
        }
        let exchange = instruction::Exchange {
            vault: key(g, 328)?,
            indices: vec![key(g, 392)?],
            buffers: vec![key(g, 424)?],
            withdraw_queue: key(g, 472)?,
        };
        let mut mints = vec![];
        for k in [m.usdc, m.mint] {
            let acc = read(&k)?;
            if acc.owner != spl_token::ID || acc.data.len() != 82 {
                return Err(invalid("classic SPL mint required"));
            }
            let token = TokenInfo::new(&k, acc, clock.epoch)?;
            if token.decimals != 6 || token.is_token_2022 {
                return Err(invalid("unsupported token format"));
            }
            mints.push(token);
        }
        let actual_supply = u64_at(&read(&m.mint)?.data, 36)?;
        if actual_supply > m.shares {
            return Err(invalid("SPL supply exceeds accounting supply"));
        }
        for k in [c.trader, exchange.indices[0], exchange.buffers[0]] {
            accounts.insert(k, required(cache, k).await?);
        }
        // Current Phoenix configuration has a single account in each arena group.
        // Explicitly fail if it expands, rather than silently omit runtime dependencies.
        for k in [exchange.indices[0], exchange.buffers[0]] {
            let a = accounts.get(&k).ok_or(invalid("missing arena"))?;
            if a.owner != PHOENIX || codec::u16_at(&a.data, 52)? != 1 {
                return Err(invalid(
                    "expanded Phoenix arena requires refreshed account discovery",
                ));
            }
        }
        let position =
            codec::position(accounts.get(&c.trader).ok_or(invalid("missing child"))?, &c)?;
        let gross = codec::gross(position, mark, cash)?;
        let liability = codec::accrued_liability(&m, gross, clock.unix_timestamp)?;
        let state = State {
            market: m,
            config: c,
            mark,
            position,
            clock,
            actual_supply,
            revision,
            gross,
            liability,
            exchange,
            accounts,
        };
        self.mints = mints;
        self.state = Some(Arc::new(state));
        // Discover all direct and nested CPI account groups from both canonical builders.
        let mut all = HashSet::new();
        for mint in [true, false] {
            let s = self.state()?;
            let amount = if mint {
                u64::try_from(s.mark.quote_per_lot.div_ceil(s.market.leverage as u128))
                    .map_err(|_| invalid("minimum lot overflow"))?
                    .saturating_mul(2)
            } else {
                s.actual_supply
            };
            if amount == 0 {
                continue;
            }
            if let Ok(ix) = instruction::build(self, amount, mint, simulation::USER, 1) {
                all.extend(ix.accounts.iter().map(|a| a.pubkey));
            }
        }
        all.extend(self.program_dependencies());
        let synthetic = [
            simulation::USER,
            instruction::ata(simulation::USER, USDC),
            instruction::ata(simulation::USER, self.mints[1].pubkey),
        ];
        let keys: Vec<_> = all.into_iter().filter(|k| !synthetic.contains(k)).collect();
        let fetched = cache.get_accounts(&keys).await?;
        if fetched.len() != keys.len() {
            self.state = None;
            return Err(invalid("cache count mismatch"));
        }
        let mut state = (*self.state.take().ok_or(invalid("state missing"))?).clone();
        for (k, v) in keys.into_iter().zip(fetched) {
            if let Some(a) = v {
                state.accounts.insert(k, a);
            }
        }
        let oracle = simulation::ExecutionOracle::load(cache, &state, self.mints[1].pubkey).await?;
        self.state = Some(Arc::new(state));
        self.simulator = Some(Arc::new(oracle));
        Ok(())
    }
    fn quote(&self, r: QuoteRequest) -> Result<QuoteResult> {
        self.direction(&r)?;
        self.state()?;
        // Exact reference outputs are available through reference_quote(). Do not
        // fabricate a concave marginal derivative from NAV for Titan's optimizer.
        Err(TradingVenueError::AmmMethodError("Ocelot production quoting is not enabled: Phoenix execution-aware quote engine and Titan curve requirements remain unresolved; use reference_quote only for local verification".into()))
    }
    fn generate_swap_instruction(&self, r: QuoteRequest, user: Pubkey) -> Result<Instruction> {
        let quote = self.reference_quote(&r)?;
        let minimum = quote.output.saturating_sub(quote.output / 100).max(1);
        self.build_with_minimum(&r, user, minimum)
    }
}

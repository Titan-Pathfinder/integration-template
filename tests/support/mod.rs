use async_trait::async_trait;
use solana_account::Account;
#[cfg(feature = "reference-simulation")]
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::Path,
};
use titan_integration_template::account_caching::{AccountCacheError, AccountsCache};
#[cfg(feature = "reference-simulation")]
use titan_integration_template::{
    ocelot::{
        self, OcelotVenue,
        instruction::ata,
        simulation::{ExecutionOracle, USER, token_account},
    },
    swap_route::{ROUTE_WEIGHT_ALL, build_swap_leg, encode_swap_route_v3_data},
    trading_venue::QuoteRequest,
};
#[cfg(feature = "reference-simulation")]
pub const ROUTER: Pubkey = Pubkey::from_str_const("T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT");
pub struct Snapshot(pub HashMap<Pubkey, Option<Account>>);
impl Snapshot {
    pub fn read(path: &Path) -> Self {
        let mut entries = HashMap::new();
        if path.is_file() {
            let mut decoder = flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap());
            let mut bytes = Vec::new();
            decoder.read_to_end(&mut bytes).unwrap();
            let packed: BTreeMap<String, Vec<u8>> = bincode::deserialize(&bytes).unwrap();
            for (key, bytes) in packed {
                entries.insert(key.parse().unwrap(), bincode::deserialize(&bytes).unwrap());
            }
            return Self(entries);
        }
        for file in std::fs::read_dir(path).expect("snapshot directory") {
            let file = file.unwrap().path();
            if file.extension().and_then(|s| s.to_str()) != Some("bin") {
                continue;
            }
            let key = file.file_stem().unwrap().to_str().unwrap().parse().unwrap();
            let account = bincode::deserialize(&std::fs::read(file).unwrap()).unwrap();
            entries.insert(key, account);
        }
        Self(entries)
    }
}
#[async_trait]
impl AccountsCache for Snapshot {
    async fn get_account(&self, key: &Pubkey) -> Result<Option<Account>, AccountCacheError> {
        Ok(self
            .0
            .get(key)
            .unwrap_or_else(|| panic!("snapshot missing {key}"))
            .clone())
    }
    async fn get_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<Account>>, AccountCacheError> {
        let mut result = Vec::new();
        for key in keys {
            result.push(self.get_account(key).await?);
        }
        Ok(result)
    }
}
#[cfg(feature = "reference-simulation")]
pub fn routed(
    v: &OcelotVenue,
    r: &QuoteRequest,
    elf: &[u8],
    reserve: Option<u64>,
) -> (ExecutionOracle, Instruction) {
    let (pda, bump) = Pubkey::find_program_address(&[b"titan_pda"], &ROUTER);
    let mut data = ocelot::discriminator("account:TitanPda").to_vec();
    data.push(bump);
    let mut oracle = v
        .execution_oracle()
        .unwrap()
        .with_program(ROUTER, elf)
        .unwrap()
        .with_account(
            pda,
            Account {
                lamports: 10_000_000,
                data,
                owner: ROUTER,
                ..Default::default()
            },
        )
        .unwrap();
    // Existing PDA ATAs exercise the template's reserved-balance protection.
    if let Some(reserve) = reserve {
        for mint in [r.input_mint, r.output_mint] {
            oracle = oracle
                .with_account(ata(pda, mint), token_account(mint, pda, reserve))
                .unwrap();
        }
    }
    let (spec, leg) = build_swap_leg(v, r, pda, 0, 1, ROUTE_WEIGHT_ALL).unwrap();
    let mut accounts = vec![
        AccountMeta::new(USER, true),
        AccountMeta::new(USER, true),
        AccountMeta::new_readonly(pda, false),
        AccountMeta::new(ata(USER, r.input_mint), false),
        AccountMeta::new(ata(USER, r.output_mint), false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_token_2022::ID, false),
        AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
    ];
    // Anchor's sentinel for each absent optional account.
    accounts.extend(vec![AccountMeta::new_readonly(ROUTER, false); 3]);
    accounts.extend([
        AccountMeta::new(ata(pda, r.input_mint), false),
        AccountMeta::new(ata(pda, r.output_mint), false),
        AccountMeta::new_readonly(r.input_mint, false),
        AccountMeta::new_readonly(r.output_mint, false),
    ]);
    accounts.extend(leg);
    (
        oracle,
        Instruction {
            program_id: ROUTER,
            accounts,
            data: encode_swap_route_v3_data(r.amount, 2, &[spec]),
        },
    )
}

/// Deliberately TEST-ONLY fixed execution prices for the two pinned snapshots.
/// These prices include the uncross observed in replay. This is NOT a general
/// Phoenix matcher and does NOT model maker risk or withdrawal health.
/// Its only purpose is to independently test the issuer math against execution.
pub struct FixtureFactory;
struct FixtureMatcher {
    curves: [Option<titan_integration_template::ocelot::curve::Curve>; 2],
    position: titan_integration_template::ocelot::codec::Position,
    funding: i64,
    bid: u64,
    ask: u64,
}
impl titan_integration_template::ocelot::quote::PhoenixQuoteFactory for FixtureFactory {
    fn prepare(
        &self,
        s: &titan_integration_template::ocelot::State,
    ) -> titan_integration_template::ocelot::codec::Result<
        std::sync::Arc<dyn titan_integration_template::ocelot::quote::PhoenixQuoter>,
    > {
        let (bid, ask) = match (s.market.id, s.clock.slot) {
            (62030, 465928618) => (901_000, 901_100),
            (63030, 465928707) => (901_200, 901_300),
            _ => {
                return Err(titan_integration_template::ocelot::codec::invalid(
                    "test matcher used outside pinned snapshots",
                ));
            }
        };
        use titan_integration_template::ocelot::quote::PhoenixQuoter;
        let native = titan_integration_template::ocelot::phoenix::NativePhoenix::prepare(s)?;
        Ok(std::sync::Arc::new(FixtureMatcher {
            // Kernel fills below remain the independent pinned exact model.
            // Titan-facing quotes use the native provider's stricter curve.
            curves: [native.curve(true).cloned(), native.curve(false).cloned()],
            position: s.position,
            funding: s.mark.funding,
            bid,
            ask,
        }))
    }
}
impl titan_integration_template::ocelot::quote::PhoenixQuoter for FixtureMatcher {
    fn curve(&self, mint: bool) -> Option<&titan_integration_template::ocelot::curve::Curve> {
        self.curves[usize::from(!mint)].as_ref()
    }

    fn quote(
        &self,
        order: &titan_integration_template::ocelot::quote::Order,
    ) -> titan_integration_template::ocelot::codec::Result<
        titan_integration_template::ocelot::quote::Fill,
    > {
        use titan_integration_template::ocelot::{codec::invalid, quote::Fill};
        if order.lots > 1000 {
            return Err(invalid("outside fixture top level"));
        }
        let quote_atoms = order.lots * if order.buy { self.ask } else { self.bid };
        let fee_atoms = (quote_atoms * 350).div_ceil(1_000_000);
        let p = self.position;
        let delta = if order.buy {
            order.lots as i128
        } else {
            -(order.lots as i128)
        };
        let realized = if order.reduce_only && p.base != 0 {
            let closing_quote = if order.buy {
                -(quote_atoms as i128)
            } else {
                quote_atoms as i128
            };
            closing_quote - p.quote as i128 * delta / p.base as i128
        } else {
            0
        };
        let collateral = p.collateral as i128
            - p.base as i128 * (self.funding as i128 - p.funding as i128)
            + order.deposit as i128
            + realized
            - fee_atoms as i128;
        let collateral_after =
            u64::try_from(collateral).map_err(|_| invalid("fixture collateral"))?;
        Ok(Fill {
            lots: order.lots,
            quote_atoms,
            fee_atoms,
            collateral_after,
            collateral_upper_bound: collateral_after,
            withdrawal_capacity: u64::MAX,
            withdrawable_after: collateral_after,
            funding_after: self.funding,
        })
    }
}

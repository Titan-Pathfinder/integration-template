use async_trait::async_trait;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::Path,
};
use titan_integration_template::{
    account_caching::{AccountCacheError, AccountsCache},
    ocelot::{
        self, OcelotVenue,
        instruction::ata,
        simulation::{ExecutionOracle, USER, token_account},
    },
    swap_route::{ROUTE_WEIGHT_ALL, build_swap_leg, encode_swap_route_v3_data},
    trading_venue::QuoteRequest,
};

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

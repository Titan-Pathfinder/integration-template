//! Local-only execution oracle. No RPC simulation, signing key, submission or network write.
use super::{
    codec::{Result, invalid},
    *,
};
use litesvm_modern::LiteSVM;
use modern_account::ReadableAccount;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_program_option::COption;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::{
    address_lookup_table::AddressLookupTableAccount,
    address_lookup_table::{
        self,
        state::{AddressLookupTable, LookupTableMeta},
    },
    message::{VersionedMessage, v0},
};
use solana_transaction::versioned::VersionedTransaction;
use spl_token::state::{Account as TokenAccount, AccountState};
use std::borrow::Cow;

pub const USER: Pubkey = Pubkey::new_from_array([211; 32]);
const LOOKUP: Pubkey = Pubkey::new_from_array([212; 32]);
#[derive(Clone)]
pub struct ExecutionOracle {
    vm: LiteSVM,
    lookup: AddressLookupTableAccount,
    pub programdata_keys: Vec<Pubkey>,
}
#[derive(Debug, Clone, Copy)]
pub struct ExecutionResult {
    pub output: u64,
    pub compute_units: u64,
    pub packet_bytes: u64,
    pub input_remaining: u64,
}
fn modern_key(key: Pubkey) -> modern_address::Address {
    modern_address::Address::new_from_array(key.to_bytes())
}
fn modern_account(a: Account) -> modern_account::Account {
    modern_account::Account {
        lamports: a.lamports,
        data: a.data,
        owner: modern_key(a.owner),
        executable: a.executable,
        rent_epoch: a.rent_epoch,
    }
}
pub fn token_account(mint: Pubkey, owner: Pubkey, amount: u64) -> Account {
    let mut data = vec![0; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner,
            amount,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut data,
    )
    .expect("fixed SPL buffer");
    Account {
        lamports: 10_000_000,
        data,
        owner: spl_token::ID,
        executable: false,
        rent_epoch: 0,
    }
}
impl ExecutionOracle {
    pub async fn load(cache: &dyn AccountsCache, s: &State, lt: Pubkey) -> Result<Self> {
        let mut vm = LiteSVM::new()
            .with_sigverify(false)
            .with_blockhash_check(false)
            .with_transaction_history(0)
            .with_compute_budget(modern_compute_budget::compute_budget::ComputeBudget {
                compute_unit_limit: 1_400_000,
                ..modern_compute_budget::compute_budget::ComputeBudget::new_with_defaults(true)
            });
        let mut programdata_keys = Vec::new();
        for id in [PROGRAM_ID, PHOENIX, EMBER] {
            let program = required(cache, id).await?;
            if !program.executable
                || program.owner != solana_sdk_ids::bpf_loader_upgradeable::ID
                || codec::bytes::<4>(&program.data, 0)? != [2, 0, 0, 0]
            {
                return Err(invalid("unsupported executable loader"));
            }
            let programdata_key = key(&program.data, 4)?;
            programdata_keys.push(programdata_key);
            let programdata = required(cache, programdata_key).await?;
            if programdata.owner != solana_sdk_ids::bpf_loader_upgradeable::ID
                || codec::bytes::<4>(&programdata.data, 0)? != [3, 0, 0, 0]
            {
                return Err(invalid("invalid program-data account"));
            }
            let elf = programdata.data.get(45..).ok_or(invalid("truncated ELF"))?;
            log::info!(
                "loading captured executable {id}: {} bytes, SHA256 {:x}",
                elf.len(),
                sha2::Sha256::digest(elf)
            );
            vm.add_program(modern_key(id), elf).map_err(|e| {
                TradingVenueError::AmmMethodError(format!("cannot load {id}: {e}").into())
            })?;
        }
        for (k, a) in &s.accounts {
            if a.executable || *k == solana_sdk_ids::sysvar::clock::ID {
                continue;
            }
            vm.set_account(modern_key(*k), modern_account(a.clone()))
                .map_err(|_| invalid("failed to load state account"))?;
        }
        vm.set_sysvar(&modern_clock::Clock {
            slot: s.clock.slot,
            epoch_start_timestamp: s.clock.epoch_start_timestamp,
            epoch: s.clock.epoch,
            leader_schedule_epoch: s.clock.leader_schedule_epoch,
            unix_timestamp: s.clock.unix_timestamp,
        });
        vm.set_account(
            modern_key(USER),
            modern_account(Account {
                lamports: 100_000_000_000,
                owner: solana_sdk_ids::system_program::ID,
                ..Default::default()
            }),
        )
        .map_err(|_| invalid("payer setup"))?;
        for mint in [USDC, lt] {
            vm.set_account(
                modern_key(instruction::ata(USER, mint)),
                modern_account(token_account(mint, USER, 0)),
            )
            .map_err(|_| invalid("token setup"))?;
        }
        let mut addresses: Vec<_> = s.accounts.keys().copied().collect();
        addresses.extend([instruction::ata(USER, USDC), instruction::ata(USER, lt)]);
        addresses.sort();
        addresses.dedup();
        if addresses.len() > 256 {
            return Err(invalid("lookup table too large"));
        }
        let table = AddressLookupTable {
            meta: LookupTableMeta::default(),
            addresses: Cow::Borrowed(&addresses),
        };
        let data = table
            .serialize_for_tests()
            .map_err(|_| invalid("lookup serialization"))?;
        vm.set_account(
            modern_key(LOOKUP),
            modern_account(Account {
                lamports: 100_000_000,
                data,
                owner: address_lookup_table::program::ID,
                executable: false,
                rent_epoch: 0,
            }),
        )
        .map_err(|_| invalid("lookup setup"))?;
        Ok(Self {
            vm,
            programdata_keys,
            lookup: AddressLookupTableAccount {
                key: LOOKUP,
                addresses,
            },
        })
    }
    /// Fork the local bank for negative tests; never writes to RPC.
    pub fn with_clock(&self, clock: &Clock) -> Self {
        let mut fork = self.clone();
        fork.vm.set_sysvar(&modern_clock::Clock {
            slot: clock.slot,
            epoch_start_timestamp: clock.epoch_start_timestamp,
            epoch: clock.epoch,
            leader_schedule_epoch: clock.leader_schedule_epoch,
            unix_timestamp: clock.unix_timestamp,
        });
        fork
    }
    /// Fork the local bank for negative tests; never writes to RPC.
    pub fn with_account(&self, key: Pubkey, account: Account) -> Result<Self> {
        let mut fork = self.clone();
        fork.vm
            .set_account(modern_key(key), modern_account(account))
            .map_err(|_| invalid("local account setup"))?;
        Ok(fork)
    }
    pub fn with_program(&self, key: Pubkey, elf: &[u8]) -> Result<Self> {
        let mut fork = self.clone();
        fork.vm
            .add_program(modern_key(key), elf)
            .map_err(|_| invalid("local program setup"))?;
        Ok(fork)
    }
    pub fn execute(
        &self,
        ix: Instruction,
        input: Pubkey,
        output: Pubkey,
        amount: u64,
    ) -> Result<ExecutionResult> {
        self.execute_with_accounts(ix, input, output, amount)
            .map(|(result, _)| result)
    }
    /// Return the executed account changes for sequential, same-bank regressions.
    /// This is reference-only tooling; no transaction is submitted.
    pub fn execute_with_accounts(
        &self,
        ix: Instruction,
        input: Pubkey,
        output: Pubkey,
        amount: u64,
    ) -> Result<(ExecutionResult, HashMap<Pubkey, Account>)> {
        let mut vm = self.vm.clone();
        vm.set_account(
            modern_key(instruction::ata(USER, input)),
            modern_account(token_account(input, USER, amount)),
        )
        .map_err(|_| invalid("simulation input setup"))?;
        let mut lookup = self.lookup.clone();
        lookup
            .addresses
            .extend(ix.accounts.iter().map(|a| a.pubkey).filter(|k| *k != USER));
        lookup.addresses.sort();
        lookup.addresses.dedup();
        if lookup.addresses.len() > 256 {
            return Err(invalid("lookup table too large"));
        }
        let table = AddressLookupTable {
            meta: LookupTableMeta::default(),
            addresses: Cow::Borrowed(&lookup.addresses),
        };
        vm.set_account(
            modern_key(LOOKUP),
            modern_account(Account {
                lamports: 100_000_000,
                data: table
                    .serialize_for_tests()
                    .map_err(|_| invalid("lookup serialization"))?,
                owner: address_lookup_table::program::ID,
                ..Default::default()
            }),
        )
        .map_err(|_| invalid("lookup setup"))?;
        let hash = solana_hash::Hash::new_from_array(vm.latest_blockhash().to_bytes());
        let message = v0::Message::try_compile(&USER, &[ix], std::slice::from_ref(&lookup), hash)
            .map_err(|_| invalid("message compilation"))?;
        let tx = VersionedTransaction {
            signatures: vec![solana_sdk::signature::Signature::default()],
            message: VersionedMessage::V0(message),
        };
        let packet_bytes =
            bincode::serialized_size(&tx).map_err(|_| invalid("transaction serialization"))?;
        if packet_bytes > 1232 {
            return Err(invalid("transaction exceeds packet size"));
        }
        let wire = bincode::serialize(&tx).map_err(|_| invalid("serialize transaction"))?;
        let tx: modern_transaction::versioned::VersionedTransaction =
            wincode::deserialize(&wire)
                .map_err(|_| invalid("decode transaction for modern runtime"))?;
        let result = vm.simulate_transaction(tx).map_err(|e| {
            TradingVenueError::AmmMethodError(
                format!(
                    "Ocelot execution rejected: {:?}; {}",
                    e.err,
                    e.meta.logs.join(" | ")
                )
                .into(),
            )
        })?;
        let destination = modern_key(instruction::ata(USER, output));
        let (_, account) = result
            .post_accounts
            .iter()
            .find(|(key, _)| *key == destination)
            .ok_or(invalid("simulation omitted output account"))?;
        let output = codec::u64_at(account.data(), 64)?;
        let source = modern_key(instruction::ata(USER, input));
        let (_, account) = result
            .post_accounts
            .iter()
            .find(|(key, _)| *key == source)
            .ok_or(invalid("simulation omitted input account"))?;
        let input_remaining = codec::u64_at(account.data(), 64)?;
        let execution = ExecutionResult {
            output,
            compute_units: result.meta.compute_units_consumed,
            packet_bytes,
            input_remaining,
        };
        let accounts = result
            .post_accounts
            .into_iter()
            .map(|(k, a)| {
                (
                    Pubkey::new_from_array(k.to_bytes()),
                    Account {
                        lamports: a.lamports(),
                        data: a.data().to_vec(),
                        owner: Pubkey::new_from_array(a.owner().to_bytes()),
                        executable: a.executable(),
                        rent_epoch: a.rent_epoch(),
                    },
                )
            })
            .collect();
        Ok((execution, accounts))
    }
}

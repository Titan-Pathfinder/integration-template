//! Capture public chain state and execute locally. Never submits a transaction.
use async_trait::async_trait;
use solana_account::Account;
use solana_pubkey::Pubkey;
use std::{collections::HashMap, path::PathBuf, sync::Mutex};
use titan_integration_template::{
    account_caching::{AccountCacheError, AccountsCache, rpc_cache::RpcClientCache},
    ocelot::{OcelotVenue, USDC},
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};

struct Capture {
    rpc: RpcClientCache,
    folder: PathBuf,
    entries: Mutex<HashMap<Pubkey, Option<Account>>>,
}
#[async_trait]
impl AccountsCache for Capture {
    async fn get_account(&self, key: &Pubkey) -> Result<Option<Account>, AccountCacheError> {
        if let Some(a) = self.entries.lock().unwrap().get(key).cloned() {
            return Ok(a);
        }
        let path = self.folder.join(format!("{key}.bin"));
        let value = if path.exists() {
            bincode::deserialize(&std::fs::read(&path).expect("read captured account"))
                .expect("decode captured account")
        } else {
            let a = self.rpc.get_account(key).await?;
            std::fs::write(path, bincode::serialize(&a).expect("serialize capture"))
                .expect("write capture");
            a
        };
        self.entries.lock().unwrap().insert(*key, value.clone());
        Ok(value)
    }
    async fn get_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<Account>>, AccountCacheError> {
        let mut missing = vec![];
        for key in keys {
            let path = self.folder.join(format!("{key}.bin"));
            if path.exists() {
                let value =
                    bincode::deserialize(&std::fs::read(path).expect("read captured account"))
                        .expect("decode captured account");
                self.entries.lock().unwrap().insert(*key, value);
            } else if !self.entries.lock().unwrap().contains_key(key) {
                missing.push(*key);
            }
        }
        if !missing.is_empty() {
            let values = self.rpc.get_accounts(&missing).await?;
            for (key, value) in missing.into_iter().zip(values) {
                std::fs::write(
                    self.folder.join(format!("{key}.bin")),
                    bincode::serialize(&value).expect("serialize capture"),
                )
                .expect("write capture");
                self.entries.lock().unwrap().insert(key, value);
            }
        }
        Ok(keys
            .iter()
            .map(|key| {
                self.entries
                    .lock()
                    .unwrap()
                    .get(key)
                    .expect("cached account")
                    .clone()
            })
            .collect())
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: probe <snapshot-dir> [market]"),
    );
    std::fs::create_dir_all(&folder)?;
    let market: Pubkey = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM".into())
        .parse()?;
    let rpc = std::env::var("SOLANA_RPC_URL")
        .expect("SOLANA_RPC_URL is required; requests are read-only");
    let rpc_url = rpc.clone();
    let cache = Capture {
        rpc: RpcClientCache::new(solana_client::nonblocking::rpc_client::RpcClient::new(rpc)),
        folder,
        entries: Mutex::new(HashMap::new()),
    };
    let account = cache.get_account(&market).await?.ok_or("market missing")?;
    let mut venue = OcelotVenue::from_account(&market, &account)?;
    venue.update_state(&cache).await?;
    if std::env::args().any(|a| a == "--refresh") {
        // Refresh all discovered dependencies, including ProgramData, in one
        // RPC response after discovery. The reported ELF hashes pin executables.
        let keys = venue.get_required_pubkeys_for_update()?;
        assert!(
            keys.len() <= 100,
            "snapshot requires a single getMultipleAccounts batch"
        );
        let fresh = solana_client::nonblocking::rpc_client::RpcClient::new(rpc_url);
        let result = fresh
            .get_multiple_accounts_with_config(
                &keys,
                solana_client::rpc_config::RpcAccountInfoConfig::default(),
            )
            .await?;
        println!(
            "state_capture_context_slot={} accounts={}",
            result.context.slot,
            keys.len()
        );
        for (key, value) in keys.into_iter().zip(result.value) {
            std::fs::write(
                cache.folder.join(format!("{key}.bin")),
                bincode::serialize(&value)?,
            )?;
            cache.entries.lock().unwrap().insert(key, value);
        }
        std::fs::write(
            cache.folder.join("capture.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"state_context_slot":result.context.slot,"market":market.to_string(),"note":"All discovered state and executable programdata in one RPC batch."}),
            )?,
        )?;
        venue.update_state(&cache).await?;
    }
    let s = venue.state()?;
    println!(
        "market={market} slot={} mark_slot={} gross={} shares={} actual={} base_lots={} mark_ticks={} revision={}",
        s.clock.slot,
        s.mark.slot,
        s.gross,
        s.market.shares,
        s.actual_supply,
        s.position.base,
        s.mark.ticks,
        s.revision
    );
    for mint in [true, false] {
        for amount in [1_000_000, 10_000_000, 50_000_000, 100_000_000] {
            let (input_mint, output_mint) = if mint {
                (USDC, s.market.mint)
            } else {
                (s.market.mint, USDC)
            };
            let r = QuoteRequest {
                input_mint,
                output_mint,
                amount,
                swap_type: SwapType::ExactIn,
            };
            let start = std::time::Instant::now();
            match venue.reference_quote(&r) {
                Ok(q) => println!(
                    "{} input={} output={} compute={} elapsed_us={}",
                    if mint { "mint" } else { "redeem" },
                    amount,
                    q.output,
                    q.compute_units,
                    start.elapsed().as_micros()
                ),
                Err(e) => println!(
                    "{} input={amount} rejected: {e}",
                    if mint { "mint" } else { "redeem" }
                ),
            }
        }
    }
    for mint in [true, false] {
        for target_lots in 2..=4u64 {
            let boundary = if mint {
                let mut x = u64::try_from(
                    (target_lots as u128 * s.mark.quote_per_lot * 10_000).div_ceil(
                        s.market.leverage as u128 * (10_000 - s.market.mint_fee_bps as u128),
                    ),
                )?;
                let lots = |x: u64| {
                    ((x - x.saturating_mul(s.market.mint_fee_bps as u64) / 10_000) as u128
                        * s.market.leverage as u128
                        / s.mark.quote_per_lot) as u64
                };
                while x > 0 && lots(x - 1) >= target_lots {
                    x -= 1;
                }
                while lots(x) < target_lots {
                    x += 1;
                }
                x
            } else {
                if s.position.base == 0 {
                    continue;
                }
                u64::try_from(
                    (target_lots as u128 * s.market.shares as u128)
                        .div_ceil(s.position.base.unsigned_abs() as u128),
                )?
            };
            let (input_mint, output_mint) = if mint {
                (USDC, s.market.mint)
            } else {
                (s.market.mint, USDC)
            };
            let q = |amount| {
                venue.reference_quote(&QuoteRequest {
                    input_mint,
                    output_mint,
                    amount,
                    swap_type: SwapType::ExactIn,
                })
            };
            if let (Ok(a), Ok(b)) = (q(boundary - 1), q(boundary)) {
                println!(
                    "lot_boundary mint={mint} lots={target_lots} input_a={} output_a={} input_b={} output_b={} delta_output={}",
                    boundary - 1,
                    a.output,
                    boundary,
                    b.output,
                    b.output as i128 - a.output as i128
                );
            }
        }
    }
    use std::io::Write;
    let packed: std::collections::BTreeMap<String, Vec<u8>> = cache
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            (
                key.to_string(),
                bincode::serialize(value).expect("capture encoding"),
            )
        })
        .collect();
    let mut encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(cache.folder.join("accounts.bin.gz"))?,
        flate2::Compression::best(),
    );
    encoder.write_all(&bincode::serialize(&packed)?)?;
    encoder.finish()?;
    Ok(())
}

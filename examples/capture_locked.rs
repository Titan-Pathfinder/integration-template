#![allow(clippy::result_large_err)]
//! Read-only geometry sampling and coherent executable capture. No transactions
//! are signed or submitted. SOLANA_RPC_URL is never printed or saved.
use async_trait::async_trait;
use solana_account::Account;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcAccountInfoConfig};
use solana_pubkey::Pubkey;
use solana_sysvar::clock::Clock;
use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    path::Path,
    sync::Arc,
    time::Duration,
};
use titan_integration_template::{
    account_caching::{AccountCacheError, AccountsCache, rpc_cache::RpcClientCache},
    ocelot::{
        EMBER, OcelotVenue, PHOENIX, PROGRAM_ID,
        diagnostics::{self, StateOnlyFactory},
    },
    trading_venue::{FromAccount, TradingVenue},
};

const MARKETS: [&str; 2] = [
    "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
    "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
];
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Snapshot(HashMap<Pubkey, Option<Account>>);
#[async_trait]
impl AccountsCache for Snapshot {
    async fn get_account(
        &self,
        k: &Pubkey,
    ) -> std::result::Result<Option<Account>, AccountCacheError> {
        self.0.get(k).cloned().ok_or_else(|| {
            AccountCacheError::FailedToFetchAccount(
                std::io::Error::other(format!("dependency outside final batch: {k}")).into(),
            )
        })
    }
    async fn get_accounts(
        &self,
        keys: &[Pubkey],
    ) -> std::result::Result<Vec<Option<Account>>, AccountCacheError> {
        let mut values = Vec::with_capacity(keys.len());
        for k in keys {
            values.push(self.get_account(k).await?);
        }
        Ok(values)
    }
}

async fn venue(cache: &dyn AccountsCache, market: Pubkey) -> Result<OcelotVenue> {
    let a = cache.get_account(&market).await?.ok_or("missing market")?;
    let mut v = OcelotVenue::from_account(&market, &a)?;
    v.set_phoenix_factory(Arc::new(StateOnlyFactory))?;
    v.update_state(cache).await?;
    v.load_reference_oracle(cache).await?;
    Ok(v)
}

fn report(cache: &Snapshot, book: Pubkey) -> Result<serde_json::Value> {
    let clock: Clock = bincode::deserialize(
        &cache.0[&solana_sdk_ids::sysvar::clock::ID]
            .as_ref()
            .ok_or("missing clock")?
            .data,
    )?;
    let r = diagnostics::inspect(
        cache.0[&diagnostics::spline_key(book)]
            .as_ref()
            .ok_or("missing spline")?,
        book,
        clock.slot,
    )?;
    Ok(serde_json::json!({
        "clock_slot": clock.slot, "geometry": format!("{:?}", r.geometry),
        "bid": r.depth.best_bid(), "ask": r.depth.best_ask(),
        "bid_lots": r.depth.declared_lots(false), "ask_lots": r.depth.declared_lots(true),
        "future_active_makers": r.future_active_makers.iter().map(|(k, slot)|
            serde_json::json!({"trader":k.to_string(), "updated":slot})).collect::<Vec<_>>()
    }))
}

fn pack(folder: &Path, cache: &Snapshot) -> Result<()> {
    let packed: BTreeMap<String, Vec<u8>> = cache
        .0
        .iter()
        .map(|(k, v)| Ok((k.to_string(), bincode::serialize(v)?)))
        .collect::<Result<_>>()?;
    let mut encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(folder.join("accounts.bin.gz"))?,
        flate2::Compression::best(),
    );
    encoder.write_all(&bincode::serialize(&packed)?)?;
    encoder.finish()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let folder = Path::new(args.get(1).ok_or(
        "usage: capture_locked <dir> [samples=100] [interval_ms=1500] [captures_per_class=2]",
    )?);
    let samples: usize = args.get(2).map_or(Ok(100), |s| s.parse())?;
    let interval: u64 = args.get(3).map_or(Ok(1500), |s| s.parse())?;
    let max: usize = args.get(4).map_or(Ok(2), |s| s.parse())?;
    std::fs::create_dir_all(folder)?;
    let url = std::env::var("SOLANA_RPC_URL").map_err(|_| "SOLANA_RPC_URL is required")?;
    let rpc = RpcClient::new(url.clone());
    let cache = RpcClientCache::new(RpcClient::new(url));
    let mut keys = vec![];
    let mut book = None;
    for market in MARKETS {
        // Never format an RPC error: transport errors may contain API keys.
        let v = venue(&cache, market.parse()?)
            .await
            .map_err(|_| "dependency discovery failed (RPC details suppressed)")?;
        let b = v.state()?.config.book;
        if book.is_some_and(|old| old != b) {
            return Err("markets no longer share one book".into());
        }
        book = Some(b);
        keys.extend(v.get_required_pubkeys_for_update()?);
    }
    keys.sort();
    keys.dedup();
    if keys.len() > 100 {
        return Err("coherent capture needs more than one RPC batch".into());
    }
    let book = book.ok_or("no book")?;
    let poll_keys = [
        solana_sdk_ids::sysvar::clock::ID,
        diagnostics::spline_key(book),
    ];
    let mut samples_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(folder.join("samples.jsonl"))?;
    let mut counts = [0usize; 4];
    let mut captures = [0usize; 4];
    let mut failures = 0;
    let mut last_slot = None;
    let mut unique_slots = 0;
    for i in 0..samples {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(interval)).await;
        }
        let result = match rpc
            .get_multiple_accounts_with_config(&poll_keys, RpcAccountInfoConfig::default())
            .await
        {
            Ok(v) => v,
            Err(_) => {
                failures += 1;
                writeln!(samples_file, "{{\"rpc_failure\":true}}")?;
                continue;
            }
        };
        let snapshot = Snapshot(poll_keys.into_iter().zip(result.value).collect());
        let mut r = report(&snapshot, book)?;
        r["context_slot"] = result.context.slot.into();
        if last_slot != Some(result.context.slot) {
            unique_slots += 1;
        }
        last_slot = Some(result.context.slot);
        let class = match r["geometry"].as_str().unwrap() {
            "Normal" => 0,
            "Locked" => 1,
            "Crossed" => 2,
            _ => 3,
        };
        counts[class] += 1;
        writeln!(samples_file, "{r}")?;
        if captures[class] >= if class == 0 { 1 } else { max } {
            continue;
        }
        let batch = match rpc
            .get_multiple_accounts_with_config(&keys, RpcAccountInfoConfig::default())
            .await
        {
            Ok(v) => v,
            Err(_) => {
                eprintln!("full capture RPC failed (details suppressed)");
                continue;
            }
        };
        let final_cache = Snapshot(keys.iter().copied().zip(batch.value).collect());
        let mut final_report = report(&final_cache, book)?;
        // Classify the final batch, never label it from an earlier poll.
        if final_report["geometry"] != r["geometry"] {
            continue;
        }
        let mut valid = true;
        for market in MARKETS {
            if venue(&final_cache, market.parse()?).await.is_err() {
                valid = false;
                break;
            }
        }
        if !valid {
            eprintln!("capture dependency/VM validation failed; discarded");
            continue;
        }
        use sha2::{Digest, Sha256};
        let mut hashes = BTreeMap::new();
        for program in [PROGRAM_ID, PHOENIX, EMBER] {
            let a = final_cache.0[&program].as_ref().ok_or("missing program")?;
            let data_key = Pubkey::new_from_array(a.data[4..36].try_into()?);
            let data = final_cache.0[&data_key]
                .as_ref()
                .ok_or("missing programdata")?;
            hashes.insert(
                program.to_string(),
                format!("{:x}", Sha256::digest(&data.data[45..])),
            );
        }
        final_report["context_slot"] = batch.context.slot.into();
        final_report["accounts"] = keys.len().into();
        final_report["executable_sha256"] = serde_json::to_value(hashes)?;
        final_report["note"] = "All state and executable accounts from one getMultipleAccounts response; original bytes, no mutations.".into();
        let capture = folder.join(format!(
            "{}-{}",
            r["geometry"].as_str().unwrap().to_lowercase(),
            batch.context.slot
        ));
        std::fs::create_dir(&capture)?;
        pack(&capture, &final_cache)?;
        std::fs::write(
            capture.join("capture.json"),
            serde_json::to_vec_pretty(&final_report)?,
        )?;
        captures[class] += 1;
        println!("captured {} at slot {}", r["geometry"], batch.context.slot);
    }
    let summary = serde_json::json!({"samples":samples, "rpc_failures":failures, "distinct_consecutive_slots":unique_slots,
        "classes":["Normal", "Locked", "Crossed", "OneSided"],
        "observations":counts, "captures":captures, "interval_ms":interval});
    std::fs::write(
        folder.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{summary}");
    Ok(())
}

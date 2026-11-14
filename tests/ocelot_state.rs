#![cfg(feature = "reference-simulation")]
//! Cache coherence, absent-account watches and executed empty-market restart.
#[allow(dead_code)]
mod support;
use async_trait::async_trait;
use solana_account::Account;
use solana_pubkey::Pubkey;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use titan_integration_template::{
    account_caching::{AccountCacheError, AccountsCache},
    ocelot::{self, OcelotVenue, USDC, instruction, simulation::USER},
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};
const FIXTURES: [(&str, &str); 4] = [
    (
        "sol3l-465928618.bin.gz",
        "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
    ),
    (
        "sol2s-465928707.bin.gz",
        "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
    ),
    (
        "sol3l-467430913.bin.gz",
        "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
    ),
    (
        "sol2s-467431351.bin.gz",
        "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
    ),
];
struct Advancing<'a> {
    early: &'a support::Snapshot,
    late: &'a support::Snapshot,
    batches: AtomicUsize,
}
#[async_trait]
impl AccountsCache for Advancing<'_> {
    async fn get_account(&self, k: &Pubkey) -> Result<Option<Account>, AccountCacheError> {
        self.late.get_account(k).await
    }
    async fn get_accounts(&self, ks: &[Pubkey]) -> Result<Vec<Option<Account>>, AccountCacheError> {
        if self.batches.fetch_add(1, Ordering::SeqCst) == 0 {
            self.early.get_accounts(ks).await
        } else {
            self.late.get_accounts(ks).await
        }
    }
}
fn request(v: &OcelotVenue, mint: bool, amount: u64) -> QuoteRequest {
    let lt = v.state().unwrap().market.mint;
    QuoteRequest {
        input_mint: if mint { USDC } else { lt },
        output_mint: if mint { lt } else { USDC },
        amount,
        swap_type: SwapType::ExactIn,
    }
}
async fn load(cache: &support::Snapshot, pool: Pubkey) -> OcelotVenue {
    let mut v = OcelotVenue::from_account(&pool, cache.0[&pool].as_ref().unwrap()).unwrap();
    v.update_state(cache).await.unwrap();
    v
}

struct Recording<'a> {
    inner: &'a support::Snapshot,
    requested: Mutex<Vec<Pubkey>>,
}
#[async_trait]
impl AccountsCache for Recording<'_> {
    async fn get_account(&self, k: &Pubkey) -> Result<Option<Account>, AccountCacheError> {
        self.requested.lock().unwrap().push(*k);
        self.inner.get_account(k).await
    }
    async fn get_accounts(&self, ks: &[Pubkey]) -> Result<Vec<Option<Account>>, AccountCacheError> {
        self.requested.lock().unwrap().extend_from_slice(ks);
        self.inner.get_accounts(ks).await
    }
}

#[tokio::test]
async fn every_requested_dependency_remains_watched_even_when_absent() {
    for (file, pool) in FIXTURES {
        let pool = pool.parse().unwrap();
        let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let mut v = OcelotVenue::from_account(&pool, cache.0[&pool].as_ref().unwrap()).unwrap();
        for _ in 0..2 {
            // Cold and warm discovery.
            let recorded = Recording {
                inner: &cache,
                requested: Mutex::new(vec![]),
            };
            v.update_state(&recorded).await.unwrap();
            let watched = v.get_required_pubkeys_for_update().unwrap();
            for key in recorded.requested.lock().unwrap().iter() {
                assert!(
                    watched.contains(key),
                    "{file}: requested but not watched: {key}"
                );
            }
        }
    }
}

#[tokio::test]
async fn market_throttle_creation_deletion_and_recreation_refresh_quotes() {
    for i in [1, 3] {
        let (file, pool) = FIXTURES[i];
        let pool = pool.parse().unwrap();
        let mut cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let mut v = load(&cache, pool).await;
        let s = v.state().unwrap();
        let bucket = Pubkey::find_program_address(
            &[b"withdraw_throttle", b"market", pool.as_ref()],
            &ocelot::PROGRAM_ID,
        )
        .0;
        assert!(cache.0[&bucket].is_none());
        let mut data = ocelot::discriminator("account:WithdrawThrottle").to_vec();
        data.extend(s.market.fees.to_bytes());
        data.extend(pool.to_bytes());
        data.extend([1, 1]); // Market scope, enabled.
        data.extend(1_000_000u64.to_le_bytes()); // Limit: 1 USDC.
        data.extend(86_400i64.to_le_bytes());
        data.extend(1_000_000u64.to_le_bytes()); // Available.
        data.extend(s.clock.unix_timestamp.to_le_bytes());
        data.extend(0u64.to_le_bytes());
        data.push(255);
        let throttle = Account {
            data,
            owner: ocelot::PROGRAM_ID,
            lamports: 2_000_000,
            ..Default::default()
        };
        let r = request(&v, false, 5_000_000);
        assert!(v.quote(r.clone()).is_ok());
        for present in [true, false, true] {
            cache.0.insert(bucket, present.then(|| throttle.clone()));
            // Emulate a host which refreshes only on a subscribed dependency.
            if v.get_required_pubkeys_for_update()
                .unwrap()
                .contains(&bucket)
            {
                v.update_state(&cache).await.unwrap();
            }
            v.load_reference_oracle(&cache).await.unwrap();
            let quote = v.quote(r.clone());
            let executed = v.reference_quote(&r);
            if present {
                assert!(quote.is_err());
                let error = format!("{:?}", executed.unwrap_err());
                assert!(error.contains("WithdrawRateLimited"));
            } else {
                assert!(quote.unwrap().expected_output <= executed.unwrap().output);
            }
            assert!(
                v.get_required_pubkeys_for_update()
                    .unwrap()
                    .contains(&bucket)
            );
        }
    }
}

#[tokio::test]
async fn executed_full_exit_allows_cold_and_warm_bootstrap() {
    for (file, pool) in FIXTURES {
        let pool = pool.parse().unwrap();
        let mut cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        // Remove fees/liabilities before the actual exit so it leaves no orphaned
        // fee equity. All supply, position and balance changes below come from SBF.
        let data = &mut cache.0.get_mut(&pool).unwrap().as_mut().unwrap().data;
        data[239..243].fill(0);
        data[281..289].fill(0);
        data[309..317].fill(0);
        let mut v = load(&cache, pool).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let r = request(&v, false, v.state().unwrap().actual_supply);
        let (_, changes) = v
            .execution_oracle()
            .unwrap()
            .execute_with_accounts(
                v.generate_swap_instruction(r.clone(), USER).unwrap(),
                r.input_mint,
                r.output_mint,
                r.amount,
            )
            .unwrap();
        cache
            .0
            .extend(changes.into_iter().map(|(k, a)| (k, Some(a))));
        for warm in [false, true] {
            let mut empty = if warm {
                v.clone()
            } else {
                OcelotVenue::from_account(&pool, cache.0[&pool].as_ref().unwrap()).unwrap()
            };
            empty.update_state(&cache).await.unwrap();
            let s = empty.state().unwrap();
            assert_eq!(
                (
                    s.actual_supply,
                    s.market.shares,
                    s.position.base,
                    s.position.collateral,
                    s.gross,
                    s.liability
                ),
                (0, 0, 0, 0, 0, 0)
            );
            assert!(
                empty
                    .get_required_pubkeys_for_update()
                    .unwrap()
                    .contains(&s.exchange.withdraw_queue)
            );
            let r = request(&empty, true, 10_000_000);
            let q = empty.quote(r.clone()).unwrap();
            empty.load_reference_oracle(&cache).await.unwrap();
            let (executed, changes) = empty
                .execution_oracle()
                .unwrap()
                .execute_with_accounts(
                    empty.generate_swap_instruction(r.clone(), USER).unwrap(),
                    r.input_mint,
                    r.output_mint,
                    r.amount,
                )
                .unwrap();
            assert!(q.expected_output > 0 && q.expected_output <= executed.output);
            let mint = empty.state().unwrap().market.mint;
            assert_eq!(
                ocelot::codec::u64_at(&changes[&mint].data, 36).unwrap(),
                executed.output
            );
            assert!(
                ocelot::codec::Market::decode(pool, &changes[&pool])
                    .unwrap()
                    .shares
                    > 0
            );
        }
    }
}
#[tokio::test]
async fn advancing_cache_uses_only_final_executed_state() {
    for (file, pool) in FIXTURES {
        let pool = pool.parse().unwrap();
        let early = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let mut v = load(&early, pool).await;
        v.load_reference_oracle(&early).await.unwrap();
        let r = request(&v, true, v.state().unwrap().gross / 10);
        let ix = v.generate_swap_instruction(r.clone(), USER).unwrap();
        let (_, changes) = v
            .execution_oracle()
            .unwrap()
            .execute_with_accounts(ix, r.input_mint, r.output_mint, r.amount)
            .unwrap();
        let mut late = support::Snapshot(early.0.clone());
        late.0
            .extend(changes.into_iter().map(|(k, a)| (k, Some(a))));
        let mut coherent = load(&late, pool).await;
        coherent.load_reference_oracle(&late).await.unwrap();
        assert!(coherent.state().unwrap().market.shares > v.state().unwrap().market.shares);
        for warm in [false, true] {
            let cache = Advancing {
                early: &early,
                late: &late,
                batches: AtomicUsize::new(0),
            };
            let mut mixed = if warm {
                v.clone()
            } else {
                OcelotVenue::from_account(&pool, early.0[&pool].as_ref().unwrap()).unwrap()
            };
            mixed.update_state(&cache).await.unwrap();
            assert_eq!(cache.batches.load(Ordering::SeqCst), 2);
            let a = mixed.state().unwrap();
            let b = coherent.state().unwrap();
            assert_eq!(
                (
                    a.market.shares,
                    a.actual_supply,
                    a.gross,
                    a.liability,
                    a.position.base
                ),
                (
                    b.market.shares,
                    b.actual_supply,
                    b.gross,
                    b.liability,
                    b.position.base
                )
            );
            assert_eq!(
                a.market.shares,
                ocelot::codec::Market::decode(pool, &a.accounts[&pool])
                    .unwrap()
                    .shares
            );
            for mint in [true, false] {
                let r = request(&mixed, mint, 1_000_000);
                let q = mixed.quote(r.clone()).unwrap();
                assert_eq!(
                    q.expected_output,
                    coherent.quote(r.clone()).unwrap().expected_output
                );
                assert!(q.expected_output <= coherent.reference_quote(&r).unwrap().output);
            }
        }
    }
}
#[tokio::test]
async fn final_read_failure_or_changed_dependencies_invalidates_previous_quote() {
    let (file, pool) = FIXTURES[2];
    let pool = pool.parse().unwrap();
    let early = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
    let v = load(&early, pool).await;
    for missing in [
        v.state().unwrap().config.trader,
        solana_sdk_ids::sysvar::clock::ID,
        instruction::pda(b"finance_v1", pool),
    ] {
        let mut late = support::Snapshot(early.0.clone());
        late.0.insert(missing, None);
        let cache = Advancing {
            early: &early,
            late: &late,
            batches: AtomicUsize::new(0),
        };
        let mut candidate = v.clone();
        assert!(candidate.update_state(&cache).await.is_err());
        assert!(!candidate.initialized());
        assert!(candidate.state().is_err());
        assert!(candidate.quote(request(&v, true, 1_000_000)).is_err());
    }
    let mut late = support::Snapshot(early.0.clone());
    late.0
        .get_mut(&ocelot::PHOENIX_GLOBAL)
        .unwrap()
        .as_mut()
        .unwrap()
        .data[328..360]
        .copy_from_slice(Pubkey::new_unique().as_ref());
    let cache = Advancing {
        early: &early,
        late: &late,
        batches: AtomicUsize::new(0),
    };
    let mut candidate = v.clone();
    let err = candidate.update_state(&cache).await.unwrap_err();
    assert!(format!("{err:?}").contains("dependencies changed"));
    assert!(!candidate.initialized());
    assert!(candidate.state().is_err());
}

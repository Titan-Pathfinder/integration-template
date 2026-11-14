#![allow(clippy::result_large_err)]
//! Spline geometry tests. The replay bridge assumes fixture fees/collateral
//! arithmetic; it does not establish maker risk or general withdrawal capacity.
#[allow(dead_code)]
mod support;
use solana_account::Account;
use solana_pubkey::Pubkey;
use std::path::Path;
use titan_integration_template::{
    account_caching::AccountsCache,
    ocelot::{
        self, OcelotVenue, PHOENIX,
        codec::{Position, Result, invalid},
        quote::{self, Fill, Order, PhoenixQuoter},
        spline::SplineDepth,
    },
    trading_venue::{FromAccount, TradingVenue},
};
#[cfg(feature = "reference-simulation")]
use titan_integration_template::{
    ocelot::USDC,
    trading_venue::{QuoteRequest, SwapType},
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

fn put64(d: &mut [u8], o: usize, n: u64) {
    d[o..o + 8].copy_from_slice(&n.to_le_bytes());
}
fn put32(d: &mut [u8], o: usize, n: u32) {
    d[o..o + 4].copy_from_slice(&n.to_le_bytes());
}
fn synthetic() -> (Pubkey, Account) {
    let book = Pubkey::new_unique();
    let mut d = vec![0; 112 + 1336];
    d[..8].copy_from_slice(&ocelot::discriminator("account:spline_collection"));
    d[8..40].copy_from_slice(book.as_ref());
    put32(&mut d, 72, 1);
    put32(&mut d, 76, 1);
    let o = 112;
    d[o + 32] = 1;
    put64(&mut d, o + 40, 100);
    put64(&mut d, o + 64, 99);
    put64(&mut d, o + 104, 1);
    put64(&mut d, o + 112, 1);
    put64(&mut d, o + 1112, 2);
    put64(&mut d, o + 1120, 3);
    for side in 0..2 {
        let r = o + 120 + side * 480;
        put64(&mut d, r, 1);
        put64(&mut d, r + 8, 3);
        put32(&mut d, r + 16, 5);
        put32(&mut d, r + 20, 999);
        put64(&mut d, r + 24, 10);
        put64(&mut d, r + 40, 1);
    }
    put32(&mut d, o + 120 + 32, 2);
    (
        book,
        Account {
            owner: PHOENIX,
            data: d,
            ..Default::default()
        },
    )
}

#[test]
fn widths_fills_expiry_hidden_liquidity_and_limits() {
    let (book, a) = synthetic();
    let depth = SplineDepth::from_account(&a, book, 100).unwrap();
    assert_eq!(depth.best_bid(), Some(97));
    assert_eq!(depth.best_ask(), Some(104));
    assert_eq!(depth.declared_lots(false), 8);
    assert_eq!(depth.declared_lots(true), 10); // Hidden take size is not incoming liquidity.
    assert_eq!(
        assert_no_alloc::assert_no_alloc(|| depth.quote_notional(8, false, 96, 2)).unwrap(),
        1542
    );
    assert_eq!(
        assert_no_alloc::assert_no_alloc(|| depth.quote_notional(8, true, 105, 2)).unwrap(),
        1670
    );
    assert!(assert_no_alloc::assert_no_alloc(|| depth.quote_notional(8, true, 104, 2)).is_err());
    assert!(depth.quote_notional(11, true, 1000, 1).is_err());
    assert_eq!(depth.quote_notional(0, true, 0, 1).unwrap(), 0);
    assert!(depth.quote_notional(1, true, 1000, 0).is_err());
    let expired = SplineDepth::from_account(&a, book, 101).unwrap();
    assert_eq!(expired.declared_lots(true), 0);
    let future = SplineDepth::from_account(&a, book, 98).unwrap();
    assert_eq!(future.excluded_future_splines, 1);
    assert_eq!(future.declared_lots(true), 0);
}

#[test]
fn multiple_splines_merge_at_price_and_crossings_reject() {
    let (book, mut a) = synthetic();
    let second = a.data[112..].to_vec();
    a.data.extend(second);
    put32(&mut a.data, 72, 2);
    put32(&mut a.data, 76, 2);
    let depth = SplineDepth::from_account(&a, book, 100).unwrap();
    assert_eq!(depth.declared_lots(true), 20);
    assert_eq!(
        depth.quote_notional(12, true, 105, 1).unwrap(),
        10 * 104 + 2 * 105
    );
    put64(&mut a.data, 112 + 1336 + 40, 110);
    let crossed = SplineDepth::from_account(&a, book, 100).unwrap();
    assert!(crossed.quote_notional(1, true, 1000, 1).is_err());
}

#[test]
fn malformed_or_unbounded_geometry_rejects() {
    let (book, a) = synthetic();
    assert!(SplineDepth::from_account(&a, Pubkey::new_unique(), 100).is_err());
    let mut b = a.clone();
    b.owner = Pubkey::new_unique();
    assert!(SplineDepth::from_account(&b, book, 100).is_err());
    for len in [0, 111, 112, 1447] {
        let mut b = a.clone();
        b.data.truncate(len);
        assert!(SplineDepth::from_account(&b, book, 100).is_err());
    }
    let mut b = a.clone();
    put64(&mut b.data, 112 + 104, 11);
    assert!(SplineDepth::from_account(&b, book, 100).is_err());
    let mut b = a.clone();
    put32(&mut b.data, 112 + 1096, 2);
    assert!(SplineDepth::from_account(&b, book, 100).is_err());
    let mut b = a.clone();
    put64(&mut b.data, 112 + 120 + 24, 11);
    assert!(SplineDepth::from_account(&b, book, 100).is_err());
    let mut b = a.clone();
    put64(&mut b.data, 112 + 600 + 8, 100_002);
    put64(&mut b.data, 112 + 600 + 24, 500_005);
    assert!(SplineDepth::from_account(&b, book, 100).is_err());
}

/// Test-only bridge into existing issuer arithmetic. No production factory.
struct GeometryBridge {
    depth: SplineDepth,
    p: Position,
    funding: i64,
    tick: u64,
    fee_micro: u32,
}
impl PhoenixQuoter for GeometryBridge {
    fn quote(&self, o: &Order) -> Result<Fill> {
        let q = self
            .depth
            .quote_notional(o.lots, o.buy, o.limit_ticks, self.tick)?;
        let fee = u64::try_from((q as u128 * self.fee_micro as u128).div_ceil(1_000_000))
            .map_err(|_| invalid("fee overflow"))?;
        let delta = if o.buy {
            o.lots as i128
        } else {
            -(o.lots as i128)
        };
        let realized = if o.reduce_only && self.p.base != 0 {
            (if o.buy { -(q as i128) } else { q as i128 })
                - self.p.quote as i128 * delta / self.p.base as i128
        } else {
            0
        };
        let collateral = self.p.collateral as i128
            - self.p.base as i128 * (self.funding as i128 - self.p.funding as i128)
            + o.deposit as i128
            + realized
            - fee as i128;
        let collateral_after = u64::try_from(collateral).map_err(|_| invalid("test collateral"))?;
        Ok(Fill {
            lots: o.lots,
            quote_atoms: q,
            fee_atoms: fee,
            collateral_after,
            collateral_upper_bound: collateral_after,
            withdrawal_capacity: u64::MAX,
            withdrawable_after: collateral_after,
            funding_after: self.funding,
        })
    }
}

async fn fixture(short: bool) -> (OcelotVenue, support::Snapshot, GeometryBridge) {
    let (file, key) = if short {
        (
            "sol2s-465928707.bin.gz",
            "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
        )
    } else {
        (
            "sol3l-465928618.bin.gz",
            "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
        )
    };
    let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
    let key = key.parse().unwrap();
    let mut v =
        OcelotVenue::from_account(&key, &cache.get_account(&key).await.unwrap().unwrap()).unwrap();
    v.update_state(&cache).await.unwrap();
    let s = v.state().unwrap();
    assert!(matches!(
        (s.market.id, s.clock.slot),
        (62030, 465928618) | (63030, 465928707)
    ));
    let key = Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &PHOENIX).0;
    let depth = SplineDepth::from_account(&s.accounts[&key], s.config.book, s.clock.slot).unwrap();
    assert_eq!(
        (depth.best_bid(), depth.best_ask()),
        if short {
            (Some(9012), Some(9013))
        } else {
            (Some(9010), Some(9011))
        }
    );
    assert_eq!(
        u32::from_le_bytes(s.accounts[&s.config.book].data[96..100].try_into().unwrap()),
        350
    );
    let bridge = GeometryBridge {
        depth,
        p: s.position,
        funding: s.mark.funding,
        tick: s.config.tick_size,
        fee_micro: 350,
    };
    assert!(
        v.initialized(),
        "adapter uses the capacity-limited native provider"
    );
    (v, cache, bridge)
}

#[tokio::test]
async fn pinned_depth_uses_account_prices_without_fixed_price_matcher() {
    for short in [false, true] {
        let (v, _, bridge) = fixture(short).await;
        let s = v.state().unwrap();
        let limits = quote::Limits::load(s).unwrap();
        assert_eq!(
            quote::quote(s, limits, &bridge, 10_000_000, true)
                .unwrap()
                .amount,
            if short { 9_752_004 } else { 9_263_379 }
        );
        assert_eq!(
            quote::quote(s, limits, &bridge, 10_000_000, false)
                .unwrap()
                .amount,
            if short { 10_196_606 } else { 10_726_165 }
        );
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn spline_geometry_against_captured_execution() {
    let (mut exact, mut under, mut rejected) = (0, 0, 0);
    let mut max_under_bps = 0.0f64;
    for short in [false, true] {
        let (mut v, cache, bridge) = fixture(short).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let s = v.state().unwrap();
        let limits = quote::Limits::load(s).unwrap();
        for mint in [true, false] {
            let boundary = match (short, mint) {
                (false, true) => 601_135,
                (false, false) => 566_096,
                (true, true) => 901_901,
                (true, false) => 845_852,
            };
            let mut sizes = vec![
                1_000_000,
                5_000_000,
                10_000_000,
                18_000_000,
                19_000_000,
                boundary - 1,
                boundary,
                boundary + 1,
            ];
            if short && !mint {
                sizes.push(s.actual_supply);
            }
            let mut seed = 7u64;
            for _ in 0..24 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                sizes.push(1_000_000 + seed % 18_000_000);
            }
            for amount in sizes {
                let req = QuoteRequest {
                    input_mint: if mint { USDC } else { s.market.mint },
                    output_mint: if mint { s.market.mint } else { USDC },
                    amount,
                    swap_type: SwapType::ExactIn,
                };
                match (
                    quote::quote(s, limits, &bridge, amount, mint),
                    v.reference_quote(&req),
                ) {
                    (Ok(q), Ok(actual)) => {
                        assert!(
                            q.amount <= actual.output,
                            "overquote short={short} mint={mint} input={amount}: {} > {}",
                            q.amount,
                            actual.output
                        );
                        if q.amount == actual.output {
                            exact += 1;
                        } else {
                            under += 1;
                        }
                        max_under_bps = max_under_bps.max(
                            (actual.output - q.amount) as f64 / actual.output as f64 * 10_000.0,
                        );
                    }
                    (Err(_), Err(_)) => rejected += 1,
                    (q, actual) => panic!(
                        "acceptance mismatch short={short} mint={mint} input={amount}: {q:?} vs {actual:?}"
                    ),
                }
            }
        }
    }
    assert!(exact + under >= 100);
    assert!(rejected > 0);
    println!(
        "SPLINE GEOMETRY + FIXTURE RISK ASSUMPTIONS: exact={exact} under={under} matching_rejections={rejected} max_under_bps={max_under_bps:.6}; two captured states only"
    );
}

#[tokio::test]
#[ignore = "run explicitly with --release; excludes production risk and withdrawal handling"]
async fn benchmark_issuer_with_spline_geometry() {
    assert!(!std::hint::black_box(cfg!(debug_assertions)));
    let (v, _, bridge) = fixture(false).await;
    let s = v.state().unwrap();
    let limits = quote::Limits::load(s).unwrap();
    let mut times = Vec::new();
    let mut checksum = 0;
    for batch in 0..100 {
        let t = std::time::Instant::now();
        for i in 0..10_000 {
            let amount = 1_000_000 + (batch * 10_000 + i) % 18_000_000;
            checksum ^= std::hint::black_box(
                quote::quote(
                    std::hint::black_box(s),
                    limits,
                    std::hint::black_box(&bridge),
                    std::hint::black_box(amount),
                    i % 2 == 0,
                )
                .unwrap()
                .amount,
            );
        }
        times.push(t.elapsed().as_nanos() as f64 / 10_000.0);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "ISSUER + SPLINE GEOMETRY + FIXTURE RISK ASSUMPTIONS: 1,000,000 calls; batch-mean p50={:.1}ns p99={:.1}ns max={:.1}ns (nearest-rank) checksum={checksum}; not production latency",
        times[49], times[98], times[99]
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
#[ignore = "read-only capture diagnostic; requires OCELOT_SPLINE_SNAPSHOT and OCELOT_SPLINE_MARKET"]
async fn fresh_spline_comparison() {
    let folder = std::env::var("OCELOT_SPLINE_SNAPSHOT").expect("capture directory required");
    let market: Pubkey = std::env::var("OCELOT_SPLINE_MARKET")
        .expect("market required")
        .parse()
        .unwrap();
    compare_snapshot(&folder, market).await;
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn spline_geometry_against_november_9_captures() {
    for (file, market) in [
        (
            "tests/fixtures/sol3l-467430913.bin.gz",
            "AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM",
        ),
        (
            "tests/fixtures/sol2s-467431351.bin.gz",
            "CNEcZ2Cf6bqQTb8r9BSyZQb8o47Qa6Fd72Z1dj5BDGqp",
        ),
    ] {
        compare_snapshot(file, market.parse().unwrap()).await;
    }
}

#[cfg(feature = "reference-simulation")]
async fn compare_snapshot(folder: &str, market: Pubkey) {
    let cache = support::Snapshot::read(Path::new(folder));
    let mut v =
        OcelotVenue::from_account(&market, &cache.get_account(&market).await.unwrap().unwrap())
            .unwrap();
    v.update_state(&cache).await.unwrap();
    v.load_reference_oracle(&cache).await.unwrap();
    let s = v.state().unwrap();
    let key = Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &PHOENIX).0;
    let depth = SplineDepth::from_account(&s.accounts[&key], s.config.book, s.clock.slot).unwrap();
    println!(
        "fresh market={} slot={} bid={:?} ask={:?} declared_bid_lots={} declared_ask_lots={} excluded_future={}",
        s.market.id,
        s.clock.slot,
        depth.best_bid(),
        depth.best_ask(),
        depth.declared_lots(false),
        depth.declared_lots(true),
        depth.excluded_future_splines
    );
    let bridge = GeometryBridge {
        depth,
        p: s.position,
        funding: s.mark.funding,
        tick: s.config.tick_size,
        fee_micro: u32::from_le_bytes(s.accounts[&s.config.book].data[96..100].try_into().unwrap()),
    };
    let limits = quote::Limits::load(s).unwrap();
    let (mut exact, mut under, mut over, mut false_accept, mut conservative_reject, mut rejected) =
        (0, 0, 0, 0, 0, 0);
    for mint in [true, false] {
        for amount in [
            1_000_000,
            5_000_000,
            10_000_000,
            50_000_000,
            100_000_000,
            500_000_000,
            1_000_000_000,
        ] {
            let req = QuoteRequest {
                input_mint: if mint { USDC } else { s.market.mint },
                output_mint: if mint { s.market.mint } else { USDC },
                amount,
                swap_type: SwapType::ExactIn,
            };
            match (
                quote::quote(s, limits, &bridge, amount, mint),
                v.reference_quote(&req),
            ) {
                (Ok(q), Ok(a)) => {
                    if q.amount == a.output {
                        exact += 1;
                    } else if q.amount < a.output {
                        under += 1;
                    } else {
                        over += 1;
                    }
                    println!(
                        "mint={mint} input={amount} estimate={} actual={} signed_error_bps={:.6}",
                        q.amount,
                        a.output,
                        (q.amount as f64 - a.output as f64) / a.output as f64 * 10_000.0
                    );
                }
                (Ok(q), Err(e)) => {
                    false_accept += 1;
                    println!(
                        "FALSE_ACCEPT mint={mint} input={amount} estimate={} actual={e}",
                        q.amount
                    );
                }
                (Err(e), Ok(a)) => {
                    conservative_reject += 1;
                    println!(
                        "CONSERVATIVE_REJECT mint={mint} input={amount} estimate={e} actual={}",
                        a.output
                    );
                }
                (Err(_), Err(_)) => {
                    rejected += 1;
                    println!("BOTH_REJECT mint={mint} input={amount}");
                }
            }
        }
    }
    println!(
        "FRESH GEOMETRY + DEFAULT FEE/COLLATERAL ASSUMPTIONS: exact={exact} under={under} over={over} false_accept={false_accept} conservative_reject={conservative_reject} matching_rejections={rejected}"
    );
    assert!(exact + under + over > 0, "no comparable successful outputs");
    assert_eq!(
        over + false_accept,
        0,
        "spline-only hypothesis failed for this capture"
    );
}

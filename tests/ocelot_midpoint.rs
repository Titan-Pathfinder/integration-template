#![allow(clippy::result_large_err)]
//! Exercise the DEFAULT production provider, not an injected midpoint factory.
#[allow(dead_code)]
mod support;
use solana_pubkey::Pubkey;
#[cfg(feature = "reference-simulation")]
use std::collections::BTreeSet;
use std::{path::Path, sync::Arc};
#[cfg(feature = "reference-simulation")]
use titan_integration_template::trading_venue::QuoteResult;
use titan_integration_template::{
    ocelot::{
        self, OcelotVenue, USDC,
        codec::{Result, u64_at},
        phoenix::NativePhoenix,
        quote::{PhoenixQuoteFactory, PhoenixQuoter},
        spline::SplineDepth,
    },
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
fn spline_key(book: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"spline", book.as_ref()], &ocelot::PHOENIX).0
}
fn inspect(a: &solana_account::Account, book: Pubkey, slot: u64) -> Result<SplineDepth> {
    SplineDepth::from_account(a, book, slot)
}
struct StrictFactory;
impl PhoenixQuoteFactory for StrictFactory {
    fn prepare(&self, s: &ocelot::State) -> Result<Arc<dyn PhoenixQuoter>> {
        Ok(Arc::new(NativePhoenix::prepare(s)?))
    }
}
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
fn put64(d: &mut [u8], o: usize, n: u64) {
    d[o..o + 8].copy_from_slice(&n.to_le_bytes());
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
async fn decoded(cache: &support::Snapshot, key: Pubkey) -> OcelotVenue {
    let mut v = OcelotVenue::from_account(&key, cache.0[&key].as_ref().unwrap()).unwrap();
    v.update_state(cache).await.unwrap();
    v
}

// Move one real maker until its bid meets another maker's ask (or vice versa).
// Accounts/risk/consumed liquidity and executables otherwise remain captured.
async fn lock(
    cache: &support::Snapshot,
    key: Pubkey,
    raise_bid: bool,
) -> (support::Snapshot, usize) {
    let v = decoded(cache, key).await;
    let s = v.state().unwrap();
    let k = spline_key(s.config.book);
    let original = cache.0[&k].as_ref().unwrap();
    let raw = inspect(original, s.config.book, s.clock.slot).unwrap();
    let n = u32::from_le_bytes(original.data[72..76].try_into().unwrap()) as usize;
    for i in 0..n {
        let mut isolated = original.clone();
        for j in 0..n {
            if j != i {
                isolated.data[112 + j * 1336 + 32] = 0;
            }
        }
        let single = inspect(&isolated, s.config.book, s.clock.slot).unwrap();
        let o = 112 + i * 1336;
        let mid = u64_at(&original.data, o + 40).unwrap();
        let new = if raise_bid {
            single
                .best_bid()
                .zip(raw.best_ask())
                .and_then(|(b, a)| mid.checked_add(a.checked_sub(b)?))
        } else {
            single
                .best_ask()
                .zip(raw.best_bid())
                .and_then(|(a, b)| mid.checked_sub(a.checked_sub(b)?))
        };
        let Some(new) = new else {
            continue;
        };
        let mut changed = support::Snapshot(cache.0.clone());
        let a = changed.0.get_mut(&k).unwrap().as_mut().unwrap();
        put64(&mut a.data, o + 40, new);
        let book = inspect(a, s.config.book, s.clock.slot).unwrap();
        if book.best_bid() == book.best_ask() {
            return (changed, i);
        }
    }
    panic!("fixture cannot form a two-maker lock");
}

async fn captured(file: &str, market: usize) -> (OcelotVenue, support::Snapshot) {
    let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
    let v = decoded(&cache, FIXTURES[market].1.parse().unwrap()).await;
    (v, cache)
}

#[tokio::test]
async fn normal_output_price_bounds_and_instructions_are_unchanged() {
    for (file, market) in FIXTURES {
        let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let v = decoded(&cache, market.parse().unwrap()).await;
        assert!(!v.uses_midpoint_approximation());
        let mut strict = v.clone();
        strict.set_phoenix_factory(Arc::new(StrictFactory)).unwrap();
        for mint in [true, false] {
            let bounds = v.bounds(u8::from(!mint), u8::from(mint)).unwrap();
            assert_eq!(
                bounds,
                strict.bounds(u8::from(!mint), u8::from(mint)).unwrap()
            );
            for a in [0, bounds.0, bounds.0 + (bounds.1 - bounds.0) / 2, bounds.1] {
                let r = request(&v, mint, a);
                let q = assert_no_alloc::assert_no_alloc(|| v.quote(r.clone())).unwrap();
                let expected = strict.quote(r.clone()).unwrap();
                assert_eq!(q.expected_output, expected.expected_output);
                assert_eq!(q.price, expected.price);
                if a > 0 {
                    let user = Pubkey::new_from_array([211; 32]);
                    assert_eq!(
                        v.generate_swap_instruction(r.clone(), user).unwrap(),
                        strict.generate_swap_instruction(r, user).unwrap()
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn default_fallback_preserves_accounts_and_scales_output_and_price_once() {
    for file in [
        "sol-locked-468615258.bin.gz",
        "sol-crossed-468615690.bin.gz",
    ] {
        for market in [0, 1] {
            let (v, cache) = captured(file, market).await;
            assert!(v.uses_midpoint_approximation());
            let s = v.state().unwrap();
            for (k, a) in &s.accounts {
                assert_eq!(a, cache.0[k].as_ref().unwrap());
            }
            let k = spline_key(s.config.book);
            let raw = inspect(&s.accounts[&k], s.config.book, s.clock.slot).unwrap();
            let sum = raw.best_bid().unwrap() as u128 + raw.best_ask().unwrap() as u128;
            for mint in [true, false] {
                // Independent unbuffered native curve on the explicitly chosen
                // side's recentered snapshot; venue still holds original state.
                let buy = if mint {
                    s.market.long
                } else {
                    s.position.base < 0
                };
                let mid = if buy { sum.div_ceil(2) } else { sum / 2 } as u64;
                let mut model = s.clone();
                let d = &mut model.accounts.get_mut(&k).unwrap().data;
                let n = u32::from_le_bytes(d[72..76].try_into().unwrap()) as usize;
                for i in 0..n {
                    let o = 112 + i * 1336;
                    if d[o + 32] == 1 && u64_at(d, o + 40).unwrap() > 0 {
                        put64(d, o + 40, mid);
                    }
                }
                let native = NativePhoenix::prepare(&model).unwrap();
                let (lo, hi) = v.bounds(u8::from(!mint), u8::from(mint)).unwrap();
                for amount in [0, lo, lo + (hi - lo) / 2, hi] {
                    let q = assert_no_alloc::assert_no_alloc(|| v.quote(request(&v, mint, amount)))
                        .unwrap();
                    let original = native.curve(mint).unwrap().quote(amount).unwrap();
                    assert_eq!(
                        q.expected_output,
                        (original.amount as u128 * 9995 / 10000) as u64
                    );
                    assert_eq!(q.price, original.price * 0.9995);
                }
            }
        }
    }
}

#[tokio::test]
async fn refresh_transitions_remove_discount_and_failed_refresh_clears_state() {
    for (file, market) in FIXTURES {
        let key = market.parse().unwrap();
        let normal = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let (mut changed, maker) = lock(&normal, key, true).await;
        let mut v = decoded(&normal, key).await;
        let r = request(&v, true, 5_000_000);
        let baseline = v.quote(r.clone()).unwrap();
        v.update_state(&changed).await.unwrap();
        assert!(v.uses_midpoint_approximation());
        let first = v.quote(r.clone()).unwrap();
        v.update_state(&changed).await.unwrap();
        assert_eq!(
            first.expected_output,
            v.quote(r.clone()).unwrap().expected_output
        );
        let k = spline_key(v.state().unwrap().config.book);
        let d = &mut changed.0.get_mut(&k).unwrap().as_mut().unwrap().data;
        let o = 112 + maker * 1336 + 40;
        let mid = u64_at(d, o).unwrap();
        put64(d, o, mid + 1);
        v.update_state(&changed).await.unwrap();
        assert!(v.uses_midpoint_approximation());
        assert!(v.quote(r.clone()).is_ok());
        // A malformed dependency cannot enable the fallback or retain old data.
        let mut invalid = support::Snapshot(changed.0.clone());
        invalid.0.get_mut(&k).unwrap().as_mut().unwrap().owner = Pubkey::default();
        assert!(v.update_state(&invalid).await.is_err());
        assert!(!v.initialized());
        assert!(!v.uses_midpoint_approximation());
        assert!(v.quote(r.clone()).is_err());
        v.update_state(&normal).await.unwrap();
        assert!(!v.uses_midpoint_approximation());
        let q = v.quote(r).unwrap();
        assert_eq!(q.expected_output, baseline.expected_output);
        assert_eq!(q.price, baseline.price);
    }
}

#[tokio::test]
async fn clock_only_expiry_and_stale_oracles_do_not_reuse_prepared_liquidity() {
    let (mut v, mut cache) = captured("sol-locked-468615258.bin.gz", 0).await;
    let s = v.state().unwrap();
    let k = spline_key(s.config.book);
    let mut clock = s.clock.clone();
    let r = request(&v, true, 1_000_000);
    let d = &mut cache.0.get_mut(&k).unwrap().as_mut().unwrap().data;
    let count = u32::from_le_bytes(d[72..76].try_into().unwrap()) as usize;
    for i in 0..count {
        let o = 112 + i * 1336;
        put64(d, o + 64, clock.slot);
        for j in 0..20 {
            put64(d, o + 120 + j * 48 + 40, 0);
        }
    }
    v.update_state(&cache).await.unwrap();
    assert!(v.quote(r.clone()).is_ok());
    clock.slot += 1;
    cache
        .0
        .get_mut(&solana_sdk_ids::sysvar::clock::ID)
        .unwrap()
        .as_mut()
        .unwrap()
        .data = bincode::serialize(&clock).unwrap();
    let _ = v.update_state(&cache).await;
    assert!(
        v.quote(r).is_err(),
        "expired splines must not retain a quote"
    );
    let (mut v, mut cache) = captured("sol-locked-468615258.bin.gz", 0).await;
    let r = request(&v, true, 1_000_000);
    let mut clock = v.state().unwrap().clock.clone();
    clock.slot += 10_000;
    cache
        .0
        .get_mut(&solana_sdk_ids::sysvar::clock::ID)
        .unwrap()
        .as_mut()
        .unwrap()
        .data = bincode::serialize(&clock).unwrap();
    assert!(v.update_state(&cache).await.is_err());
    assert!(!v.initialized());
    assert!(v.quote(r).is_err());
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn captured_outputs_and_known_policy_limitations_match_execution() {
    for file in [
        "sol-locked-468615258.bin.gz",
        "sol-crossed-468615690.bin.gz",
    ] {
        for market in [0, 1] {
            let (mut v, cache) = captured(file, market).await;
            v.load_reference_oracle(&cache).await.unwrap();
            for mint in [true, false] {
                for amount in [1_000_000, 5_000_000] {
                    let r = request(&v, mint, amount);
                    let q = v.quote(r.clone()).unwrap();
                    let e = v.reference_quote(&r).unwrap();
                    assert!(q.expected_output <= e.output);
                    let ix = v
                        .build_with_minimum(&r, ocelot::simulation::USER, q.expected_output)
                        .unwrap();
                    assert_eq!(
                        v.execution_oracle()
                            .unwrap()
                            .execute(ix, r.input_mint, r.output_mint, r.amount)
                            .unwrap()
                            .output,
                        e.output
                    );
                }
            }
        }
    }
    // Do NOT turn these known approximation errors into passing safety claims.
    for (index, width, amount, expected, actual) in [
        (2, 1, 406_715_349, 0, 429_724_357),
        (0, 4, 346_554_022, 320_644_986, 320_592_122),
        (2, 8, 480_611_111, 507_192_066, 506_741_449),
    ] {
        let (file, market) = FIXTURES[index];
        let key = market.parse().unwrap();
        let original = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
        let (mut changed, maker) = lock(&original, key, true).await;
        let v = decoded(&changed, key).await;
        let k = spline_key(v.state().unwrap().config.book);
        let d = &mut changed.0.get_mut(&k).unwrap().as_mut().unwrap().data;
        let o = 112 + maker * 1336 + 40;
        let mid = u64_at(d, o).unwrap();
        put64(d, o, mid + width);
        let mut v = decoded(&changed, key).await;
        v.load_reference_oracle(&changed).await.unwrap();
        let r = request(&v, true, amount);
        let q = v.quote(r.clone()).unwrap();
        let e = v.reference_quote(&r).unwrap();
        assert_eq!(e.output, actual);
        if width == 1 {
            assert!(q.expected_output <= e.output, "buy midpoint rounds up");
        } else {
            assert_eq!(q.expected_output, expected);
            assert!(
                q.expected_output > e.output,
                "five bps is not a universal output bound"
            );
            let ix = v
                .build_with_minimum(&r, ocelot::simulation::USER, q.expected_output)
                .unwrap();
            assert!(
                v.execution_oracle()
                    .unwrap()
                    .execute(ix, r.input_mint, r.output_mint, r.amount)
                    .is_err()
            );
        }
    }
}

#[cfg(feature = "reference-simulation")]
fn check_chord(a: &QuoteResult, b: &QuoteResult) {
    assert!(b.expected_output >= a.expected_output);
    assert!(b.price > 0.0 && b.price <= a.price * (1.0 + 1e-5));
    if a.amount == b.amount {
        return;
    }
    let dx = (b.amount - a.amount) as f64;
    let slope = (b.expected_output - a.expected_output) as f64 / dx;
    assert!(slope <= a.price * (1.0 + 1e-5) + 2.0 / dx);
    assert!(slope >= b.price * (1.0 - 1e-5) - 2.0 / dx);
}

#[cfg(feature = "reference-simulation")]
async fn sweep(cache: &support::Snapshot, key: Pubkey, label: &str, stride: u64, captured: bool) {
    let mut v = decoded(cache, key).await;
    v.load_reference_oracle(cache).await.unwrap();
    for mint in [true, false] {
        let Ok((lo, hi)) = v.bounds(u8::from(!mint), u8::from(mint)) else {
            println!(
                "MIDPOINT {}",
                serde_json::json!({"case":label,"mint":mint,"declined":true})
            );
            continue;
        };
        let s = v.state().unwrap();
        let first = ocelot::quote::order(s, lo, mint).unwrap().lots;
        let last = ocelot::quote::order(s, hi, mint).unwrap().lots;
        assert!(last - first < 1_000_000);
        let edge = |lots: u64| -> u64 {
            if mint {
                let deposit =
                    (lots as u128 * s.mark.quote_per_lot).div_ceil(s.market.leverage as u128);
                (((deposit - 1) * 10_000) / (10_000 - s.market.mint_fee_bps as u128) + 1) as u64
            } else {
                (lots as u128 * s.market.shares as u128)
                    .div_ceil(s.position.base.unsigned_abs() as u128) as u64
            }
        };
        let mut amounts = BTreeSet::new();
        let mut execute = BTreeSet::new();
        for lots in first.max(1)..=last + 1 {
            let e = edge(lots);
            let next = edge(lots + 1);
            for a in [e.saturating_sub(1), e, e + 1, e + (next - e) / 2] {
                if a >= lo && a <= hi {
                    amounts.insert(a);
                    if (lots - first).is_multiple_of(stride)
                        || lots <= first + 2
                        || lots >= last.saturating_sub(2)
                    {
                        execute.insert(a);
                    }
                }
            }
        }
        for a in [
            lo,
            hi,
            500_000,
            1_000_000,
            10_000_000,
            100_000_000,
            500_000_000,
            1_000_000_000,
        ] {
            if a >= lo && a <= hi {
                amounts.insert(a);
                execute.insert(a);
            }
        }
        let mut seed = 0x69123498u64;
        for _ in 0..64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = lo + seed % (hi - lo + 1);
            amounts.insert(a);
            execute.insert(a);
        }
        let (mut quotes, mut comparisons, mut over, mut failures, mut holes) =
            (0u64, 0u64, 0u64, 0u64, 0u64);
        let (mut max_over, mut max_under) = (0f64, 0f64);
        let mut worst = None;
        let mut previous = None;
        for amount in amounts {
            let r = request(&v, mint, amount);
            let Ok(q) = assert_no_alloc::assert_no_alloc(|| v.quote(r.clone())) else {
                holes += 1;
                continue;
            };
            quotes += 1;
            if let Some(p) = &previous {
                check_chord(p, &q);
            }
            if execute.contains(&amount) {
                match v.reference_quote(&r) {
                    Ok(e) => {
                        comparisons += 1;
                        if q.expected_output > e.output {
                            over += 1;
                            let bps =
                                (q.expected_output - e.output) as f64 * 10_000.0 / e.output as f64;
                            if bps > max_over {
                                max_over = bps;
                                worst = Some((amount, q.expected_output, e.output));
                            }
                        } else {
                            max_under = max_under.max(
                                (e.output - q.expected_output) as f64 * 10_000.0 / e.output as f64,
                            );
                        }
                    }
                    Err(e) => {
                        failures += 1;
                        println!("MIDPOINT_EXECUTION_ERROR {label} {mint} {amount}: {e}");
                    }
                }
            }
            previous = Some(q);
        }
        println!(
            "MIDPOINT {}",
            serde_json::json!({"case":label,"mint":mint,"lo":lo,"hi":hi,"stride":stride,"quotes":quotes,"comparisons":comparisons,"overquotes":over,"execution_failures":failures,"range_holes":holes,"max_over_bps":max_over,"max_under_bps":max_under,"worst":worst})
        );
        assert_eq!(holes, 0);
        if captured {
            assert_eq!((over, failures), (0, 0), "captured-state regression");
        }
    }
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
#[ignore = "explicit production-provider execution sweep; controlled states report known quote errors"]
async fn production_execution_sweep() {
    let controlled = std::env::var("MIDPOINT_SWEEP").is_ok_and(|x| x == "controlled");
    if !controlled {
        for file in [
            "sol-locked-468615258.bin.gz",
            "sol-crossed-468615690.bin.gz",
        ] {
            for market in [0, 1] {
                let (_, cache) = captured(file, market).await;
                sweep(
                    &cache,
                    FIXTURES[market].1.parse().unwrap(),
                    &format!("captured/{file}/market={market}"),
                    1,
                    true,
                )
                .await;
            }
        }
    } else {
        for (file, market) in FIXTURES {
            let key = market.parse().unwrap();
            let original = support::Snapshot::read(&Path::new("tests/fixtures").join(file));
            for raise in [false, true] {
                let (locked, maker) = lock(&original, key, raise).await;
                let v = decoded(&locked, key).await;
                let k = spline_key(v.state().unwrap().config.book);
                for width in [0, 1, 2, 4, 8, 16, 32] {
                    let mut changed = support::Snapshot(locked.0.clone());
                    let d = &mut changed.0.get_mut(&k).unwrap().as_mut().unwrap().data;
                    let o = 112 + maker * 1336 + 40;
                    let mid = u64_at(d, o).unwrap();
                    put64(d, o, if raise { mid + width } else { mid - width });
                    sweep(
                        &changed,
                        key,
                        &format!("controlled/{file}/raise={raise}/cross_ticks={width}"),
                        64,
                        false,
                    )
                    .await;
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "release timing only; full default quote and cached account refresh"]
async fn benchmark_midpoint_quote_and_refresh() {
    use std::{hint::black_box, time::Instant};
    assert!(!black_box(cfg!(debug_assertions)));
    for (file, market) in [
        (FIXTURES[0].0, 0),
        ("sol-locked-468615258.bin.gz", 0),
        ("sol-crossed-468615690.bin.gz", 0),
        ("sol-crossed-468615690.bin.gz", 1),
    ] {
        let (mut v, cache) = captured(file, market).await;
        let requests = [request(&v, true, 1_000_000), request(&v, false, 1_000_000)];
        let mut quote_ns = Vec::new();
        let mut refresh_us = Vec::new();
        for _ in 0..3 {
            v.update_state(&cache).await.unwrap();
        }
        for _ in 0..100 {
            let now = Instant::now();
            for i in 0..10_000 {
                black_box(v.quote(black_box(requests[i % 2].clone())).unwrap());
            }
            quote_ns.push(now.elapsed().as_nanos() as f64 / 10_000.0);
            let now = Instant::now();
            v.update_state(&cache).await.unwrap();
            refresh_us.push(now.elapsed().as_secs_f64() * 1e6);
        }
        quote_ns.sort_by(f64::total_cmp);
        refresh_us.sort_by(f64::total_cmp);
        println!(
            "MIDPOINT_BENCH {}",
            serde_json::json!({"file":file,"market":market,"quote_batch_mean_ns":{"p50":quote_ns[49],"p99":quote_ns[98],"max":quote_ns[99]},"refresh_us":{"p50":refresh_us[49],"p99":refresh_us[98],"max":refresh_us[99]},"samples":100})
        );
        assert!(quote_ns[98] < 1_000.0);
    }
}

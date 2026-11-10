#![allow(clippy::result_large_err)] // Preserve Titan's allocation-free public error type.
//! Issuer-kernel tests. The bounded fixture matcher is not production Phoenix code.
#[allow(dead_code)]
mod support;
use std::{path::Path, sync::Arc};
use titan_integration_template::{
    account_caching::AccountsCache,
    ocelot::{
        OcelotVenue, USDC,
        quote::{self, Fill, Order, PhoenixQuoter},
    },
    trading_venue::{FromAccount, QuoteRequest, SwapType, TradingVenue},
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

async fn fixture(short: bool) -> (OcelotVenue, support::Snapshot) {
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
    let mut venue =
        OcelotVenue::from_account(&key, &cache.get_account(&key).await.unwrap().unwrap()).unwrap();
    venue.update_state(&cache).await.unwrap();
    assert!(
        venue.initialized(),
        "bundled native provider prepares from cache"
    );
    venue
        .set_phoenix_factory(Arc::new(support::FixtureFactory))
        .unwrap();
    assert!(venue.initialized());
    (venue, cache)
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

#[tokio::test]
async fn native_quotes_match_pinned_outputs_without_vm() {
    for (short, mint_expected, redeem_expected, mint_boundary, redeem_boundary) in [
        (false, 9_263_379, 10_726_165, 601_135, 566_096),
        (true, 9_752_004, 10_196_606, 901_901, 845_852),
    ] {
        let (v, _) = fixture(short).await;
        for (mint, expected, boundary) in [
            (true, mint_expected, mint_boundary),
            (false, redeem_expected, redeem_boundary),
        ] {
            let r = request(&v, mint, 10_000_000);
            let q = assert_no_alloc::assert_no_alloc(|| v.quote(r.clone())).unwrap();
            assert_eq!(q.expected_output, expected);
            assert_eq!(q.amount, r.amount);
            assert!(!q.not_enough_liquidity);
            let a = v.quote(request(&v, mint, boundary - 1)).unwrap();
            let b = v.quote(request(&v, mint, boundary)).unwrap();
            assert!(
                a.expected_output > b.expected_output,
                "must preserve real lot-boundary steps"
            );
            let zero = assert_no_alloc::assert_no_alloc(|| v.quote(request(&v, mint, 0))).unwrap();
            assert_eq!(zero.expected_output, 0);
            assert!(zero.price.is_finite() && zero.price > 0.0);
            assert!(assert_no_alloc::assert_no_alloc(|| v.quote(request(&v, mint, 1))).is_err());
            let ix = v
                .generate_swap_instruction(r, solana_pubkey::Pubkey::new_unique())
                .unwrap();
            assert_eq!(
                ix.program_id,
                titan_integration_template::ocelot::PROGRAM_ID
            );
        }
    }
}

#[tokio::test]
async fn native_state_refresh_invalidates_or_rebuilds_matcher() {
    let (mut v, mut cache) = fixture(false).await;
    let before = v
        .quote(request(&v, true, 10_000_000))
        .unwrap()
        .expected_output;
    v.update_state(&cache).await.unwrap();
    assert!(v.initialized());
    assert_eq!(
        v.quote(request(&v, true, 10_000_000))
            .unwrap()
            .expected_output,
        before
    );
    struct Reject;
    impl quote::PhoenixQuoteFactory for Reject {
        fn prepare(
            &self,
            _: &titan_integration_template::ocelot::State,
        ) -> titan_integration_template::ocelot::codec::Result<Arc<dyn PhoenixQuoter>> {
            Err(titan_integration_template::ocelot::codec::invalid(
                "unsupported Phoenix state",
            ))
        }
    }
    assert!(v.set_phoenix_factory(Arc::new(Reject)).is_err());
    assert!(!v.initialized());
    assert!(v.update_state(&cache).await.is_err());
    assert!(
        v.state().is_err(),
        "failure during native preparation must invalidate all state"
    );
    v.set_phoenix_factory(Arc::new(support::FixtureFactory))
        .unwrap();
    v.update_state(&cache).await.unwrap();
    let req = request(&v, true, 10_000_000);
    cache
        .0
        .insert(titan_integration_template::ocelot::PERP_MAP, None);
    assert!(v.update_state(&cache).await.is_err());
    assert!(!v.initialized());
    assert!(assert_no_alloc::assert_no_alloc(|| v.quote(req)).is_err());
}

struct Fixed(Fill);
impl PhoenixQuoter for Fixed {
    fn quote(&self, _: &Order) -> titan_integration_template::ocelot::codec::Result<Fill> {
        Ok(self.0)
    }
}

#[tokio::test]
async fn native_rejects_bad_fills_caps_and_costs() {
    use titan_integration_template::ocelot::quote::PhoenixQuoteFactory;
    let (v, _) = fixture(false).await;
    let s = v.state().unwrap();
    let limits = quote::Limits::load(s).unwrap();
    let matcher = support::FixtureFactory.prepare(s).unwrap();
    for mint in [true, false] {
        let order = quote::order(s, 10_000_000, mint).unwrap();
        let good = matcher.quote(&order).unwrap();
        let mut bad = good;
        bad.lots -= 1;
        assert!(quote::quote(s, limits, &Fixed(bad), 10_000_000, mint).is_err());
        bad = good;
        bad.funding_after += 1;
        assert!(quote::quote(s, limits, &Fixed(bad), 10_000_000, mint).is_err());
        bad = good;
        bad.fee_atoms = s.gross;
        assert!(quote::quote(s, limits, &Fixed(bad), 10_000_000, mint).is_err());
        let mut paused = s.clone();
        paused.market.paused = true;
        assert!(quote::quote(&paused, limits, matcher.as_ref(), 10_000_000, mint).is_err());
    }
    let mut capped = s.clone();
    capped.market.deposit_cap = 9_999_999;
    assert!(quote::quote(&capped, limits, matcher.as_ref(), 10_000_000, true).is_err());
    capped.market.deposit_cap = 0;
    capped.market.equity_cap = s.gross - s.liability;
    assert!(quote::quote(&capped, limits, matcher.as_ref(), 10_000_000, true).is_err());
    let mut depleted = limits;
    depleted.withdrawal_capacity = 1;
    assert!(quote::quote(s, depleted, matcher.as_ref(), 10_000_000, false).is_err());
    let mut unavailable = matcher
        .quote(&quote::order(s, 10_000_000, false).unwrap())
        .unwrap();
    unavailable.withdrawable_after = 0;
    assert!(quote::quote(s, limits, &Fixed(unavailable), 10_000_000, false).is_err());
    assert!(v.quote(request(&v, false, s.actual_supply + 1)).is_err());
}

#[tokio::test]
async fn cash_exits_bootstrap_and_abandoned_supply() {
    let (v, _) = fixture(false).await;
    let mut s = v.state().unwrap().clone();
    let mut limits = quote::Limits::load(&s).unwrap();
    s.position.base = 0;
    s.position.quote = 0;
    s.position.collateral = 0;
    s.market.shares = 1_000_000;
    s.actual_supply = 1_000_000;
    s.gross = 2_000_000;
    s.liability = 1_000;
    s.market.redeem_fee_bps = 100;
    limits.cash = 2_000_000;
    limits.withdrawal_capacity = u64::MAX;
    let empty = Fill {
        lots: 0,
        quote_atoms: 0,
        fee_atoms: 0,
        collateral_after: 0,
        collateral_upper_bound: 0,
        withdrawal_capacity: u64::MAX,
        withdrawable_after: 0,
        funding_after: s.mark.funding,
    };
    assert_eq!(
        quote::quote(&s, limits, &Fixed(empty), 500_000, false)
            .unwrap()
            .amount,
        989_505
    );
    assert_eq!(
        quote::quote(&s, limits, &Fixed(empty), 1_000_000, false)
            .unwrap()
            .amount,
        1_979_010
    );
    // Burning half the SPL supply does not transfer abandoned claims to the final holder.
    s.actual_supply = 500_000;
    assert_eq!(
        quote::quote(&s, limits, &Fixed(empty), 500_000, false)
            .unwrap()
            .amount,
        989_505
    );
    s.position.base = 3;
    assert_eq!(quote::order(&s, 499_999, false).unwrap().lots, 1);
    assert_eq!(quote::order(&s, 500_000, false).unwrap().lots, 2);
    s.position.base = 0;
    s.actual_supply = 0;
    s.market.shares = 0;
    assert!(
        quote::quote(&s, limits, &Fixed(empty), 10_000_000, true).is_err(),
        "orphaned equity requires recovery"
    );
    s.gross = 0;
    s.liability = 0;
    limits.cash = 0;
    let order = quote::order(&s, 10_000_000, true).unwrap();
    let fill = Fill {
        lots: order.lots,
        quote_atoms: (order.lots as u128 * s.mark.quote_per_lot) as u64,
        fee_atoms: 0,
        collateral_after: order.deposit,
        collateral_upper_bound: order.deposit,
        withdrawal_capacity: u64::MAX,
        withdrawable_after: 0,
        funding_after: s.mark.funding,
    };
    assert_eq!(
        quote::quote(&s, limits, &Fixed(fill), 10_000_000, true)
            .unwrap()
            .amount,
        order.deposit
    );
    // A large abandoned per-share value must not mint dust below the rounding bound.
    s.market.shares = 1;
    s.gross = 10_000_001;
    assert!(quote::quote(&s, limits, &Fixed(fill), 10_000_000, true).is_err());
}

#[tokio::test]
async fn refreshed_global_throttle_uses_fractional_capacity() {
    let (v, _) = fixture(false).await;
    let mut s = v.state().unwrap().clone();
    let now = s.clock.unix_timestamp;
    let d = &mut s
        .accounts
        .get_mut(&titan_integration_template::ocelot::global_bucket())
        .unwrap()
        .data;
    d[73] = 1;
    for (offset, value) in [(74, 100u64), (82, 3), (90, 20), (106, 2)] {
        d[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    d[98..106].copy_from_slice(&(now - 1).to_le_bytes());
    assert_eq!(quote::Limits::load(&s).unwrap().withdrawal_capacity, 54);
    s.clock.unix_timestamp = now + 3;
    assert_eq!(quote::Limits::load(&s).unwrap().withdrawal_capacity, 100);
    s.clock.unix_timestamp = now - 2;
    assert!(quote::Limits::load(&s).is_err());
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn issuer_math_matches_deployed_execution_over_sizes_and_lot_edges() {
    let mut count = 0;
    let mut rejections = 0;
    for short in [false, true] {
        let (mut v, cache) = fixture(short).await;
        v.load_reference_oracle(&cache).await.unwrap();
        for mint in [true, false] {
            let mut sizes = vec![1_000_000, 10_000_000, 19_000_000];
            if mint || !short {
                sizes.extend([50_000_000, 100_000_000]);
            }
            let boundary: u64 = match (short, mint) {
                (false, true) => 601_135,
                (false, false) => 566_096,
                (true, true) => 901_901,
                (true, false) => 845_852,
            };
            sizes.extend([boundary - 1, boundary, boundary + 1]);
            if short && !mint {
                sizes.push(v.state().unwrap().actual_supply);
            }
            // Deterministic random amounts within the fixture matcher's bounded top level.
            let mut seed = 7u64;
            for _ in 0..24 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                sizes.push(1_000_000 + seed % 18_000_000);
            }
            for amount in sizes {
                let r = request(&v, mint, amount);
                match (v.quote(r.clone()), v.reference_quote(&r)) {
                    (Ok(native), Ok(actual)) => {
                        assert_eq!(
                            native.expected_output, actual.output,
                            "short={short} mint={mint} amount={amount}"
                        );
                        count += 1;
                    }
                    (Err(_), Err(_)) => rejections += 1,
                    (native, actual) => panic!(
                        "acceptance mismatch short={short} mint={mint} amount={amount}: {native:?} vs {actual:?}"
                    ),
                }
            }
        }
    }
    assert!(count >= 100);
    assert!(
        rejections > 0,
        "include post-withdrawal finance margin rejection"
    );
    println!(
        "issuer parity: {count} exact outputs and {rejections} matching rejections against captured deployed programs"
    );
}

#[tokio::test]
#[ignore = "run explicitly with --release; benchmark excludes real Phoenix matching"]
async fn benchmark_issuer_with_fixture_matcher() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "benchmark requires --release"
    );
    let (v, _) = fixture(false).await;
    let mut samples = Vec::new();
    let mut checksum = 0u64;
    for batch in 0..100 {
        let start = std::time::Instant::now();
        for i in 0..10_000 {
            let r = request(
                &v,
                i % 2 == 0,
                1_000_000 + ((batch * 10_000 + i) % 18_000_000),
            );
            checksum ^= std::hint::black_box(&v)
                .quote(std::hint::black_box(r))
                .unwrap()
                .expected_output;
        }
        samples.push(start.elapsed().as_nanos() as f64 / 10_000.0);
    }
    std::hint::black_box(checksum);
    samples.sort_by(f64::total_cmp);
    println!(
        "ISSUER + FIXTURE MATCHER ONLY: 1,000,000 calls, batch-mean p50={:.1}ns p99={:.1}ns max={:.1}ns (nearest-rank); checksum={checksum}",
        samples[49], samples[98], samples[99]
    );
    assert!(
        samples[99] < 1000.0,
        "issuer alone exhausts Titan's entire time budget"
    );
}

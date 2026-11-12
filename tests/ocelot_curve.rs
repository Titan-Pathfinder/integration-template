#![allow(clippy::result_large_err)]
//! Deterministic boundary coverage supplements Titan's randomized shared suite.
#[allow(dead_code)]
mod support;
use std::path::Path;
use titan_integration_template::{
    ocelot::{
        OcelotVenue, USDC,
        phoenix::NativePhoenix,
        quote::{self, PhoenixQuoter},
    },
    trading_venue::{
        FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue, bounds::find_boundaries,
    },
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

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
async fn fixture(i: usize) -> (OcelotVenue, support::Snapshot) {
    let (name, market) = FIXTURES[i];
    let cache = support::Snapshot::read(&Path::new("tests/fixtures").join(name));
    let pool = market.parse().unwrap();
    let mut v = OcelotVenue::from_account(&pool, cache.0[&pool].as_ref().unwrap()).unwrap();
    v.update_state(&cache).await.unwrap();
    (v, cache)
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
fn check_chord(a: &QuoteResult, b: &QuoteResult) {
    assert!(
        a.expected_output <= b.expected_output,
        "downward step at {} -> {}",
        a.amount,
        b.amount
    );
    assert!(b.price > 0.0 && b.price <= a.price * (1.0 + 1e-5));
    if a.amount == b.amount {
        return;
    }
    let chord = (b.expected_output - a.expected_output) as f64 / (b.amount - a.amount) as f64;
    // Exactly the shared suite's price-consistency tolerances, including flats.
    let atol = 2.0 / (b.amount - a.amount) as f64;
    assert!(
        chord <= a.price * (1.0 + 1e-5) + atol,
        "chord {chord} above {} at {} -> {}",
        a.price,
        a.amount,
        b.amount
    );
    assert!(
        chord >= b.price * (1.0 - 1e-5) - atol,
        "chord {chord} below {} at {} -> {}",
        b.price,
        a.amount,
        b.amount
    );
}

#[tokio::test]
async fn every_fixture_lot_edge_is_monotone_concave_and_below_native_output() {
    let mut cases = 0;
    for (i, _) in FIXTURES.iter().enumerate() {
        let (v, _) = fixture(i).await;
        let s = v.state().unwrap();
        let limits = quote::Limits::load(s).unwrap();
        let native = NativePhoenix::prepare(s).unwrap();
        for mint in [true, false] {
            let raw = |amount| {
                let out = quote::quote(s, limits, &native, amount, mint)?;
                let r = request(&v, mint, amount);
                Ok(QuoteResult {
                    input_mint: r.input_mint,
                    output_mint: r.output_mint,
                    amount,
                    expected_output: out.amount,
                    not_enough_liquidity: false,
                    price: out.price,
                })
            };
            let (lo, hi) = find_boundaries(&raw).unwrap();
            let first_lot = quote::order(s, lo, mint).unwrap().lots;
            let last_lot = quote::order(s, hi, mint).unwrap().lots;
            let mut amounts = vec![lo, hi];
            for lots in first_lot..=last_lot + 1 {
                let edge = if mint {
                    let deposit =
                        (lots as u128 * s.mark.quote_per_lot).div_ceil(s.market.leverage as u128);
                    (((deposit - 1) * 10_000) / (10_000 - s.market.mint_fee_bps as u128) + 1) as u64
                } else {
                    (lots as u128 * s.market.shares as u128)
                        .div_ceil(s.position.base.unsigned_abs() as u128) as u64
                };
                // Both sides of every lot jump, plus interior points.
                amounts.extend([
                    edge.saturating_sub(2),
                    edge.saturating_sub(1),
                    edge,
                    edge + 1,
                    edge + 2,
                ]);
            }
            let mut seed = 0x58123498_u64;
            for _ in 0..4096 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                amounts.push(lo + seed % (hi - lo + 1));
            }
            if !mint {
                amounts.extend([s.actual_supply.saturating_sub(1), s.actual_supply]);
            }
            amounts.sort_unstable();
            amounts.dedup();
            let mut previous = None;
            let mut worst = (0u64, 0f64, 0u64);
            for amount in amounts.into_iter().filter(|&a| a >= lo) {
                let Ok(bound) = raw(amount) else {
                    assert!(v.quote(request(&v, mint, amount)).is_err());
                    continue;
                };
                let q = assert_no_alloc::assert_no_alloc(|| v.quote(request(&v, mint, amount)))
                    .unwrap_or_else(|e| {
                        panic!(
                            "{} mint={mint} input={amount} raw={} rejected: {e}",
                            FIXTURES[i].0, bound.expected_output
                        )
                    });
                assert!(
                    q.expected_output <= bound.expected_output,
                    "curve overstates native bound"
                );
                if let Some(p) = &previous {
                    check_chord(p, &q);
                }
                let under = bound.expected_output - q.expected_output;
                let bps = under as f64 * 10_000.0 / bound.expected_output as f64;
                if bps > worst.1 {
                    worst = (under, bps, amount);
                }
                previous = Some(q);
                cases += 1;
            }
            // Reproduce the shared suite's geometric chords across the domain.
            let mut last = v.quote(request(&v, mint, lo)).unwrap();
            for k in 1..=64 {
                let amount = ((lo as f64).ln()
                    + ((hi as f64).ln() - (lo as f64).ln()) * k as f64 / 64.0)
                    .exp() as u64;
                let q = v.quote(request(&v, mint, amount.clamp(lo, hi))).unwrap();
                check_chord(&last, &q);
                last = q;
            }
            let spot = v.quote(request(&v, mint, 0)).unwrap();
            assert_eq!(spot.expected_output, 0);
            assert!(spot.price.is_finite() && spot.price > 0.0);
            eprintln!(
                "{} mint={mint}: range={lo}..{hi} lots={first_lot}..{last_lot} segments={} worst extra underquote={} atoms / {:.6} bp at {}",
                FIXTURES[i].0,
                native.curve(mint).unwrap().segment_count(),
                worst.0,
                worst.1,
                worst.2
            );
        }
    }
    assert!(cases > 200_000);
    eprintln!("curve: {cases} accepted edge/interior comparisons, no upward bound violations");
}

#[tokio::test]
async fn titan_reported_drop_and_nav_price_mismatch_are_fixed() {
    let (v, _) = fixture(0).await;
    let a = v.quote(request(&v, true, 601_134)).unwrap();
    let b = v.quote(request(&v, true, 601_135)).unwrap();
    check_chord(&a, &b);
    assert!(b.expected_output <= 556_843);
    let a = v.quote(request(&v, true, 1_000_000)).unwrap();
    let b = v.quote(request(&v, true, 1_000_000_000)).unwrap();
    check_chord(&a, &b);
    assert!((b.price - 0.92632).abs() < 0.00001);
}

/// Independent whole-lot execution arithmetic over synthetic depth. Risk is
/// unlimited here so the test isolates fee/share rounding and cost sign changes.
struct CostMatcher<'a> {
    state: &'a titan_integration_template::ocelot::State,
    depth: &'a titan_integration_template::ocelot::spline::SplineDepth,
    fee: u64,
}
impl PhoenixQuoter for CostMatcher<'_> {
    fn quote(
        &self,
        o: &quote::Order,
    ) -> titan_integration_template::ocelot::codec::Result<quote::Fill> {
        let s = self.state;
        let q = self
            .depth
            .quote_notional(o.lots, o.buy, o.limit_ticks, s.config.tick_size)?;
        let fee = (q as u128 * self.fee as u128).div_ceil(1_000_000) as u64
            + if self.fee == 0 {
                0
            } else {
                o.lots.saturating_sub(1)
            };
        let delta = if o.buy {
            o.lots as i128
        } else {
            -(o.lots as i128)
        };
        let realized = if o.reduce_only && s.position.base != 0 {
            (if o.buy { -(q as i128) } else { q as i128 })
                - s.position.quote as i128 * delta / s.position.base as i128
        } else {
            0
        };
        let collateral = s.position.collateral as i128 + o.deposit as i128 + realized - fee as i128;
        let collateral = u64::try_from(collateral)
            .map_err(|_| titan_integration_template::ocelot::codec::invalid("test collateral"))?;
        Ok(quote::Fill {
            lots: o.lots,
            quote_atoms: q,
            fee_atoms: fee,
            collateral_after: collateral,
            collateral_upper_bound: collateral,
            withdrawal_capacity: u64::MAX,
            withdrawable_after: collateral,
            funding_after: s.mark.funding,
        })
    }
}

fn synthetic_depth(
    book: solana_pubkey::Pubkey,
) -> titan_integration_template::ocelot::spline::SplineDepth {
    titan_integration_template::ocelot::spline::SplineDepth::from_account(
        &synthetic_spline(book),
        book,
        100,
    )
    .unwrap()
}

fn synthetic_spline(book: solana_pubkey::Pubkey) -> solana_account::Account {
    use titan_integration_template::ocelot;
    let mut data = vec![0u8; 112 + 1336];
    data[..8].copy_from_slice(&ocelot::discriminator("account:spline_collection"));
    data[8..40].copy_from_slice(book.as_ref());
    data[72..76].copy_from_slice(&1u32.to_le_bytes());
    data[76..80].copy_from_slice(&1u32.to_le_bytes());
    data[112 + 32] = 1;
    let put = |d: &mut [u8], off: usize, n: u64| d[off..off + 8].copy_from_slice(&n.to_le_bytes());
    put(&mut data, 112 + 40, 10_000);
    put(&mut data, 112 + 64, 100);
    for side in 0..2 {
        put(&mut data, 112 + 104 + side * 8, 1);
        put(&mut data, 112 + 1112 + side * 8, 1);
        let region = 112 + 120 + side * 480;
        put(&mut data, region + 8, 8);
        data[region + 16..region + 20].copy_from_slice(&2u32.to_le_bytes());
        put(&mut data, region + 24, 16);
        put(&mut data, region + 40, u64::MAX);
    }
    solana_account::Account {
        data,
        owner: ocelot::PHOENIX,
        ..Default::default()
    }
}

#[tokio::test]
async fn locked_and_crossed_books_still_invalidate_quotes() {
    use titan_integration_template::ocelot::PHOENIX;
    for second_mid in [10_002u64, 10_003] {
        let (mut v, mut cache) = fixture(0).await;
        let book = v.state().unwrap().config.book;
        let mut a = synthetic_spline(book);
        let second = a.data[112..].to_vec();
        a.data.extend(second);
        a.data[72..76].copy_from_slice(&2u32.to_le_bytes());
        a.data[76..80].copy_from_slice(&2u32.to_le_bytes());
        a.data[112 + 1336 + 40..112 + 1336 + 48].copy_from_slice(&second_mid.to_le_bytes());
        let key =
            solana_pubkey::Pubkey::find_program_address(&[b"spline", book.as_ref()], &PHOENIX).0;
        cache.0.insert(key, Some(a));
        let r = request(&v, true, 5_000_000);
        let error = v.update_state(&cache).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("crossed splines require Phoenix slide-price support")
        );
        assert!(!v.initialized());
        assert!(v.quote(r).is_err());
    }
}

#[tokio::test]
async fn synthetic_fee_cost_sign_cash_bootstrap_and_abandoned_supply_bounds() {
    use titan_integration_template::ocelot::curve::Curve;
    let (v, _) = fixture(0).await;
    let mut comparisons = 0;
    for long in [true, false] {
        for fee_bps in [0, 1, 100] {
            for phoenix_fee in [0, 351, 701] {
                for mode in ["normal", "cash", "bootstrap", "abandoned"] {
                    let mut s = v.state().unwrap().clone();
                    s.market.long = long;
                    s.market.leverage = if long { 3 } else { 2 };
                    s.market.mint_fee_bps = fee_bps;
                    s.market.redeem_fee_bps = fee_bps * 10;
                    s.market.deposit_cap = 0;
                    s.market.equity_cap = 0;
                    s.market.shares = 9_000_013;
                    s.actual_supply = s.market.shares - u64::from(mode == "abandoned");
                    s.config.tick_size = 100;
                    // Early fills favorable and later fills adverse when fees
                    // are small; larger fees move that sign-change boundary.
                    s.mark.ticks = if long { 10_004 } else { 9_996 };
                    s.mark.quote_per_lot = s.mark.ticks as u128 * 100;
                    s.position.base = if long { 6 } else { -6 };
                    s.position.quote =
                        -(s.position.base as i128 * s.mark.quote_per_lot as i128) as i64;
                    s.position.collateral = 10_000_019;
                    s.gross = s.position.collateral as u64;
                    s.liability = 17;
                    if mode == "cash" || mode == "bootstrap" {
                        s.position.base = 0;
                        s.position.quote = 0;
                        s.position.collateral = 0;
                    }
                    if mode == "bootstrap" {
                        s.market.shares = 0;
                        s.actual_supply = 0;
                        s.gross = 0;
                        s.liability = 0;
                    }
                    let depth = synthetic_depth(s.config.book);
                    let matcher = CostMatcher {
                        state: &s,
                        depth: &depth,
                        fee: phoenix_fee,
                    };
                    let limits = quote::Limits {
                        cash: if mode == "cash" { s.gross } else { 0 },
                        min_venue_equity: 0,
                        max_venue_leverage_bps: 100_000,
                        withdrawal_capacity: u64::MAX,
                    };
                    for mint in [true, false] {
                        let Some(curve) =
                            Curve::from_spline_depth(&s, &depth, phoenix_fee, mint).unwrap()
                        else {
                            continue;
                        };
                        let mut amounts = vec![
                            1,
                            10_000,
                            100_000,
                            1_000_000,
                            s.actual_supply.saturating_sub(1),
                            s.actual_supply,
                        ];
                        for lot in 1..=16u128 {
                            let edge = if mint {
                                let d = (lot * s.mark.quote_per_lot)
                                    .div_ceil(s.market.leverage as u128);
                                (d - 1) * 10_000 / (10_000 - s.market.mint_fee_bps as u128) + 1
                            } else if s.position.base != 0 {
                                (lot * s.market.shares as u128)
                                    .div_ceil(s.position.base.unsigned_abs() as u128)
                            } else {
                                lot * 100_000
                            } as u64;
                            amounts.extend([
                                edge.saturating_sub(2),
                                edge.saturating_sub(1),
                                edge,
                                edge + 1,
                                edge + 2,
                                edge + 79,
                            ]);
                        }
                        amounts.sort_unstable();
                        amounts.dedup();
                        let mut previous: Option<(u64, quote::Output)> = None;
                        for amount in amounts.into_iter().filter(|&x| x > 0) {
                            let Ok(raw) = quote::quote(&s, limits, &matcher, amount, mint) else {
                                continue;
                            };
                            let q = match curve.quote(amount) {
                                Ok(q) => q,
                                Err(e) => {
                                    // A sub-atom conservative estimate cannot be
                                    // returned as a positive executable quote.
                                    assert!(
                                        mode == "cash" && !mint && raw.amount <= 2,
                                        "long={long} fee={fee_bps}/{phoenix_fee} mode={mode} mint={mint} input={amount} raw={}: {e}",
                                        raw.amount
                                    );
                                    continue;
                                }
                            };
                            assert!(
                                q.amount <= raw.amount,
                                "long={long} fee={fee_bps}/{phoenix_fee} mode={mode} mint={mint} input={amount}: {} > {}",
                                q.amount,
                                raw.amount
                            );
                            if let Some((px, pq)) = previous {
                                assert!(
                                    q.amount >= pq.amount && q.price <= pq.price,
                                    "input {px} -> {amount}"
                                );
                                let chord = (q.amount - pq.amount) as f64 / (amount - px) as f64;
                                let tol = 2.0 / (amount - px) as f64;
                                assert!(
                                    chord <= pq.price * (1.0 + 1e-5) + tol
                                        && chord >= q.price * (1.0 - 1e-5) - tol
                                );
                            }
                            previous = Some((amount, q));
                            comparisons += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(comparisons > 5000);
    eprintln!("synthetic curve accounting: {comparisons} accepted comparisons");
}

#[tokio::test]
async fn advertised_bounds_have_no_seeded_finance_gaps() {
    use rand::{Rng, SeedableRng};
    for (i, _) in FIXTURES.iter().enumerate() {
        let (v, _) = fixture(i).await;
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x7174_616e_5345_4544);
        for (a, b) in v.directions_num() {
            let mint = a == 0;
            let (lo, hi) = v.bounds(a, b).unwrap();
            eprintln!("{} mint={mint} advertised={lo}..{hi}", FIXTURES[i].0);
            for _ in 0..200 {
                let r: f64 = rng.random();
                let amount = (((lo as f64).ln() + r * ((hi as f64).ln() - (lo as f64).ln())).exp()
                    as u64)
                    .clamp(lo, hi);
                v.quote(request(&v, mint, amount)).unwrap_or_else(|e| {
                    panic!(
                        "{} mint={mint} range={lo}..{hi} at {amount}: {e}",
                        FIXTURES[i].0
                    )
                });
            }
        }
    }
}

#[tokio::test]
async fn mint_bounds_exclude_real_finance_holes() {
    let (mut v, mut cache) = fixture(3).await;
    let finance = titan_integration_template::ocelot::instruction::pda(b"finance_v1", v.pool_id);
    cache.0.get_mut(&finance).unwrap().as_mut().unwrap().data[111..115]
        .copy_from_slice(&20_200u32.to_le_bytes()); // Review prose said 22,000; probe uses 20,200.
    v.update_state(&cache).await.unwrap();
    let hole = 866_219;
    assert!(v.quote(request(&v, true, hole)).is_err());
    let (lo, hi) = v.bounds(0, 1).unwrap();
    assert!(hole < lo || hole > hi, "known mint hole inside {lo}..{hi}");
    #[cfg(feature = "reference-simulation")]
    {
        v.load_reference_oracle(&cache).await.unwrap();
        assert!(v.reference_quote(&request(&v, true, hole)).is_err());
    }
}

#[tokio::test]
async fn one_failed_curve_does_not_disable_the_other_direction() {
    let (mut v, mut cache) = fixture(2).await;
    // Near-exhausted holder equity makes the redemption curve's slope negative,
    // while minting still adds equity and the venue's finance collateral is intact.
    let liability = v.state().unwrap().gross - 1;
    cache.0.get_mut(&v.pool_id).unwrap().as_mut().unwrap().data[309..317]
        .copy_from_slice(&liability.to_le_bytes());
    v.update_state(&cache).await.unwrap();
    let native = NativePhoenix::prepare(v.state().unwrap()).unwrap();
    assert!(native.curve(false).is_none());
    assert!(native.curve(true).is_some());
    let q = v.quote(request(&v, true, 1_000_000)).unwrap();
    assert!(q.expected_output > 0);
    assert!(v.quote(request(&v, false, 1_000_000)).is_err());
    #[cfg(feature = "reference-simulation")]
    {
        v.load_reference_oracle(&cache).await.unwrap();
        let actual = v.reference_quote(&request(&v, true, 1_000_000)).unwrap();
        assert!(q.expected_output <= actual.output);
    }
}

const FINANCE_CASES: [&str; 10] = [
    "captured",
    "target+20",
    "target+200",
    "current-50",
    "current+50",
    "target+10pct",
    "minimum-equity-120pct",
    "cash-20pct",
    "zero-issuer-fees",
    "zero-phoenix-fee",
];

async fn finance_fixture(i: usize, case: usize) -> (OcelotVenue, support::Snapshot) {
    use titan_integration_template::ocelot::instruction::pda;
    let (mut v, mut cache) = fixture(i).await;
    let s = v.state().unwrap().clone();
    let limits = quote::Limits::load(&s).unwrap();
    let current = (s.position.base.unsigned_abs() as u128 * s.mark.quote_per_lot * 10_000)
        .div_ceil((s.gross - limits.cash) as u128) as u32;
    let target = s.market.leverage as u32 * 10_000;
    let k = match case {
        0 => limits.max_venue_leverage_bps,
        1 => target + 20,
        2 => target + 200,
        3 => current - 50,
        5 => target * 11 / 10,
        _ => current + 50,
    };
    let finance = pda(b"finance_v1", v.pool_id);
    let f = &mut cache.0.get_mut(&finance).unwrap().as_mut().unwrap().data;
    f[111..115].copy_from_slice(&k.to_le_bytes());
    if case == 6 {
        f[115..123].copy_from_slice(&(s.gross * 12 / 10).to_le_bytes());
    }
    if case == 7 {
        let cash = s.gross / 5;
        f[42] = 1;
        f[51..59].copy_from_slice(&cash.to_le_bytes());
        let market = &mut cache.0.get_mut(&v.pool_id).unwrap().as_mut().unwrap().data;
        let flags = u64::from_le_bytes(market[317..325].try_into().unwrap()) | 1024;
        market[317..325].copy_from_slice(&flags.to_le_bytes());
        cache
            .0
            .get_mut(&pda(b"market_cash_v1", v.pool_id))
            .unwrap()
            .as_mut()
            .unwrap()
            .data[64..72]
            .copy_from_slice(&cash.to_le_bytes());
    }
    if case == 8 {
        cache.0.get_mut(&v.pool_id).unwrap().as_mut().unwrap().data[241..245].fill(0);
    }
    if case == 9 {
        cache
            .0
            .get_mut(&s.config.book)
            .unwrap()
            .as_mut()
            .unwrap()
            .data[96..100]
            .fill(0);
    }
    v.update_state(&cache).await.unwrap();
    (v, cache)
}

#[tokio::test]
async fn stressed_finance_ranges_have_no_lot_or_endpoint_gaps() {
    let mut accepted = 0;
    let mut unavailable = 0;
    for (i, _) in FIXTURES.iter().enumerate() {
        for (case, label) in FINANCE_CASES.iter().enumerate() {
            let (v, _) = finance_fixture(i, case).await;
            let s = v.state().unwrap();
            for mint in [true, false] {
                let bounds = if mint { v.bounds(0, 1) } else { v.bounds(1, 0) };
                let (lo, hi) = match bounds {
                    Ok(range) => range,
                    Err(e) => {
                        assert_ne!(case, 0, "captured direction must remain available");
                        unavailable += 1;
                        eprintln!("{} {label} mint={mint}: no range ({e})", FIXTURES[i].0);
                        continue;
                    }
                };
                eprintln!("{} {label} mint={mint}: {lo}..{hi}", FIXTURES[i].0);
                let first = quote::order(s, lo, mint).unwrap().lots;
                let last = quote::order(s, hi, mint).unwrap().lots;
                let mut inputs = vec![lo, hi];
                for lot in first..=last + 1 {
                    let edge = if mint {
                        let d = (lot as u128 * s.mark.quote_per_lot)
                            .div_ceil(s.market.leverage as u128);
                        ((d - 1) * 10_000 / (10_000 - s.market.mint_fee_bps as u128) + 1) as u64
                    } else {
                        (lot as u128 * s.market.shares as u128)
                            .div_ceil(s.position.base.unsigned_abs() as u128)
                            as u64
                    };
                    inputs.extend([edge.saturating_sub(1), edge, edge + 1]);
                }
                for d in 0..4096 {
                    inputs.extend([lo.saturating_add(d), hi.saturating_sub(d)]);
                }
                let mut seed = 7u64;
                for _ in 0..4096 {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    inputs.push(lo + seed % (hi - lo + 1));
                    let t = (seed >> 11) as f64 / (1u64 << 53) as f64;
                    inputs.push(
                        ((lo as f64).ln() + t * ((hi as f64).ln() - (lo as f64).ln())).exp() as u64,
                    );
                }
                inputs.sort_unstable();
                inputs.dedup();
                let mut previous = None;
                for x in inputs.into_iter().filter(|&x| x >= lo && x <= hi) {
                    let q = v.quote(request(&v, mint, x)).unwrap_or_else(|e| {
                        panic!(
                            "{} {label} mint={mint} [{lo},{hi}] hole={x}: {e}",
                            FIXTURES[i].0
                        )
                    });
                    if let Some(ref p) = previous {
                        check_chord(p, &q);
                    }
                    previous = Some(q);
                    accepted += 1;
                }
            }
        }
    }
    assert!(accepted > 500_000);
    eprintln!(
        "finance range stress: {accepted} accepted comparisons; {unavailable}/80 directions unavailable"
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn stressed_finance_range_endpoints_execute_without_overquote() {
    let mut accepted = 0;
    for (i, _) in FIXTURES.iter().enumerate() {
        for (case, label) in FINANCE_CASES.iter().enumerate() {
            let (mut v, cache) = finance_fixture(i, case).await;
            v.load_reference_oracle(&cache).await.unwrap();
            for mint in [true, false] {
                let Ok((lo, hi)) = (if mint { v.bounds(0, 1) } else { v.bounds(1, 0) }) else {
                    continue;
                };
                let mut inputs = vec![lo, lo + 1, hi - 1, hi];
                for part in 1..8 {
                    inputs.push(lo + (hi - lo) * part / 8);
                }
                inputs.sort_unstable();
                inputs.dedup();
                for x in inputs.into_iter().filter(|&x| x >= lo && x <= hi) {
                    let r = request(&v, mint, x);
                    let q = v.quote(r.clone()).unwrap();
                    let actual = v.reference_quote(&r).unwrap_or_else(|e| {
                        panic!("{} {label} mint={mint} input={x}: {e}", FIXTURES[i].0)
                    });
                    assert!(q.expected_output <= actual.output);
                    accepted += 1;
                }
            }
        }
    }
    assert!(accepted > 400);
    eprintln!(
        "finance range execution: {accepted} accepted comparisons, no overquotes or execution failures"
    );
}

#[cfg(feature = "reference-simulation")]
#[tokio::test]
async fn curve_breakpoints_and_sizes_underquote_captured_execution() {
    let mut compared = 0;
    let mut within_one_bp = 0;
    let mut worst = (0.0f64, String::new());
    for (i, _) in FIXTURES.iter().enumerate() {
        let (mut v, cache) = fixture(i).await;
        v.load_reference_oracle(&cache).await.unwrap();
        let native = NativePhoenix::prepare(v.state().unwrap()).unwrap();
        for mint in [true, false] {
            let (lo, hi) = if mint { v.bounds(0, 1) } else { v.bounds(1, 0) }.unwrap();
            let mut sizes = vec![
                lo,
                hi,
                1_000_000,
                5_000_000,
                10_000_000,
                100_000_000,
                500_000_000,
                1_000_000_000,
            ];
            for point in native.curve(mint).unwrap().breakpoints() {
                sizes.extend([point.saturating_sub(1), point, point.saturating_add(1)]);
            }
            if !mint {
                sizes.extend([
                    v.state().unwrap().actual_supply.saturating_sub(1),
                    v.state().unwrap().actual_supply,
                ]);
            }
            for k in 0..64 {
                let x = ((lo as f64).ln() + ((hi as f64).ln() - (lo as f64).ln()) * k as f64 / 63.0)
                    .exp() as u64;
                sizes.push(x.clamp(lo, hi));
            }
            sizes.sort_unstable();
            sizes.dedup();
            for amount in sizes.into_iter().filter(|&x| x > 0) {
                let r = request(&v, mint, amount);
                let Ok(q) = v.quote(r.clone()) else {
                    continue;
                };
                let executed = v.reference_quote(&r).unwrap();
                assert!(
                    q.expected_output <= executed.output,
                    "{} mint={mint} input={amount}: {} > {}",
                    FIXTURES[i].0,
                    q.expected_output,
                    executed.output
                );
                let error = (executed.output - q.expected_output) as f64 * 10_000.0
                    / executed.output as f64;
                within_one_bp += usize::from(error <= 1.0);
                if error > worst.0 {
                    worst = (
                        error,
                        format!("{} mint={mint} input={amount}", FIXTURES[i].0),
                    );
                }
                compared += 1;
            }
        }
    }
    assert!(compared > 500);
    eprintln!(
        "curve execution: {compared} accepted comparisons, {within_one_bp} within 1 bp; worst {:.6} bp ({})",
        worst.0, worst.1
    );
}

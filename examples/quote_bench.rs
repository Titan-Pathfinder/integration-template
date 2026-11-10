use std::time::Instant;
use titan_integration_template::coffer::math::CofferMath;
use titan_integration_template::coffer::state::CofferPool;
use titan_integration_template::coffer::swap::quote_exact_in;

fn main() {
    let mut pool = CofferPool {
        token_count: 2,
        swap_fee_rate: 3_000,
        protocol_fee_rate: 2_000,
        ..Default::default()
    };
    for (i, w) in [(0usize, 8_000u64), (1, 2_000)] {
        pool.tokens[i].config.normalized_weight = w;
        pool.tokens[i].dynamics.virtual_balance = 5_000_000_000_000;
        pool.tokens[i].dynamics.actual_balance = 5_000_000_000_000;
    }
    let n = 200_000u64;
    // log-uniform amounts from 1 to 1e12
    let amounts: Vec<u64> = (0..n)
        .map(|i| (10f64.powf(12.0 * (i as f64) / (n as f64))) as u64 + 1)
        .collect();
    let mut acc = 0u64;
    let t = Instant::now();
    for &a in &amounts {
        acc = acc.wrapping_add(
            CofferMath::calc_out_given_in(
                5_000_000_000_000,
                8_000,
                5_000_000_000_000,
                2_000,
                a,
                5_000_000_000_000,
                9,
                9,
            )
            .unwrap(),
        );
    }
    println!(
        "calc_out_given_in avg: {:.1} ns  ({acc})",
        t.elapsed().as_nanos() as f64 / n as f64
    );
    let t = Instant::now();
    for &a in &amounts {
        acc = acc.wrapping_add(
            quote_exact_in(&pool, a, 0, 1, 9, 9, 0)
                .unwrap()
                .amount_out_user,
        );
    }
    println!(
        "quote_exact_in (no surge) avg: {:.1} ns ({acc})",
        t.elapsed().as_nanos() as f64 / n as f64
    );
    // surge on
    let cfg = &mut pool.tokens[0].config;
    cfg.max_selloff_pct = 10_000;
    cfg.max_selloff_period_length = 3600;
    cfg.variable_fee_threshold_pct = 10;
    cfg.variable_fee_slope_low_pct = 100;
    cfg.variable_fee_slope_mid_pct = 1000;
    cfg.variable_fee_slope_high_pct = 3000;
    cfg.variable_fee_kink_pct = 50;
    let t = Instant::now();
    let mut ok = 0;
    for &a in &amounts {
        if let Ok(q) = quote_exact_in(&pool, a, 0, 1, 9, 9, 0) {
            acc = acc.wrapping_add(q.amount_out_user);
            ok += 1;
        }
    }
    println!(
        "quote_exact_in (surge, {ok} ok) avg: {:.1} ns ({acc})",
        t.elapsed().as_nanos() as f64 / n as f64
    );
    let t = Instant::now();
    for &a in &amounts {
        acc = acc.wrapping_add(((a as f64 / 5e12).powf(4.0) * 1e9) as u64);
    }
    println!(
        "f64 powf avg: {:.1} ns ({acc})",
        t.elapsed().as_nanos() as f64 / n as f64
    );
}

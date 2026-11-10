//! Diagnostic: replicate the shared suite's mean-value-theorem check and print
//! every violating chord (pool, direction, interval, sizes) instead of the first.
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_pubkey::Pubkey;
use titan_integration_template::account_caching::rpc_cache::RpcClientCache;
use titan_integration_template::coffer_venue::CofferVenue;
use titan_integration_template::example::RaydiumAmmVenue;
use titan_integration_template::trading_venue::{
    FromAccount, QuoteRequest, SwapType, TradingVenue,
};

fn geometric_grid(lb: u64, ub: u64, n: usize) -> Vec<u64> {
    let ln_lo = (lb as f64).ln();
    let ln_hi = (ub as f64).ln();
    let mut points: Vec<u64> = (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64;
            ((ln_lo + t * (ln_hi - ln_lo)).exp() as u64).clamp(lb, ub)
        })
        .collect();
    points.sort();
    points.dedup();
    points
}

async fn check<V: TradingVenue + FromAccount>(rpc_url: &str, pool: Pubkey) {
    let rpc = RpcClient::new(rpc_url.to_string());
    let account = rpc.get_account(&pool).await.unwrap();
    let mut venue = V::from_account(&pool, &account).unwrap();
    let cache = RpcClientCache::new(rpc);
    venue.update_state(&cache).await.unwrap();
    for (i, j) in venue.directions_num() {
        let (lb, ub) = venue.bounds(i, j).unwrap();
        let im = venue.get_token(i as usize).unwrap().pubkey;
        let om = venue.get_token(j as usize).unwrap().pubkey;
        let q = |x: u64| {
            venue
                .quote(QuoteRequest {
                    input_mint: im,
                    output_mint: om,
                    amount: x,
                    swap_type: SwapType::ExactIn,
                })
                .unwrap()
        };
        let grid = geometric_grid(lb, ub, 64);
        let mut viol = 0;
        let mut viol_in_aware = 0;
        let mut worst: f64 = 0.0;
        let mut max_a = 0u64;
        let mut first = String::new();
        for w in grid.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (qa, qb) = (q(a), q(b));
            if qb.expected_output <= qa.expected_output {
                continue;
            }
            let chord = (qb.expected_output - qa.expected_output) as f64 / (b - a) as f64;
            let atol = 2.0 / (b - a) as f64;
            let hi = qa.price * (1.0 + 1e-5) + atol;
            let lo = qb.price * (1.0 - 1e-5) - atol;
            if chord > hi || chord < lo {
                viol += 1;
                max_a = max_a.max(a);
                let rel = if chord > hi {
                    chord / qa.price - 1.0
                } else {
                    1.0 - chord / qb.price
                };
                if rel > worst {
                    worst = rel;
                    first = format!(
                        "[{a},{b}] chord {chord:.3} pa {:.3} pb {:.3}",
                        qa.price, qb.price
                    );
                }
            }
            // input-rounding-aware slack: one input atom of ceil'd fee is worth `price` output atoms
            let atol2 = (2.0 + qa.price) / (b - a) as f64;
            if chord > qa.price * (1.0 + 1e-5) + atol2 || chord < qb.price * (1.0 - 1e-5) - atol2 {
                viol_in_aware += 1;
            }
        }
        println!(
            "{pool} dir {i}->{j} lb {lb} ub {ub} spot {:.4e} violations {viol}/{} (max a {max_a}) with_input_atom_slack {viol_in_aware} worst_rel {worst:.2e} {first}",
            q(0).price,
            grid.len() - 1
        );
    }
}

#[tokio::main]
async fn main() {
    let rpc = std::env::var("SOLANA_RPC_URL").unwrap();
    println!("== raydium reference");
    check::<RaydiumAmmVenue>(
        &rpc,
        "Bzc9NZfMqkXR6fz1DBph7BDf9BroyEf6pnzESP7v5iiw"
            .parse()
            .unwrap(),
    )
    .await;
    println!("== coffer pools");
    for p in [
        "5dDezuaofYZUBdab8gSWY3ys86VbMrTxsRuf3ayqe8GJ",
        "CSgrEBxsghBsY1oXycBEhZVTd5PEbZuWFDZHHrtFF7yb",
        "BN4wpuvb4TyNbNigMW9cWYYrBfCH2NcquGp4AKxe8Lxm",
        "AL4yxA4o28c3dPwDSQm7VN1fxNeVfim13qzKkJfs4cKe",
    ] {
        check::<CofferVenue>(&rpc, p.parse().unwrap()).await;
    }
}

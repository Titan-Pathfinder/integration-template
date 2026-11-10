//! Route simulation (`swap_route_v3` through TitanPDA in LiteSVM) against
//! every static pool of the local-validator stand
//! (`scripts/local-stand/stand.json`, built by `scripts/local-stand/up.sh`).
//! Prints a per-pool table; SKIPs when the stand or SOLANA_RPC_URL is absent.

mod common;

use std::path::Path;

use common::{RouteConfig, run_swap_route};
use solana_pubkey::Pubkey;
use titan_integration_template::coffer_venue::{COFFER_PROGRAM_ID, CofferVenue};
use titan_integration_template::local_stand::{QuotableDirections, StandManifest};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn route_simulation_over_the_local_stand() {
    let Some(manifest) = StandManifest::load() else {
        eprintln!(
            "SKIP route_simulation_over_the_local_stand: no {} — run scripts/local-stand/up.sh",
            StandManifest::path().display()
        );
        return;
    };
    if std::env::var("SOLANA_RPC_URL").is_err() {
        eprintln!(
            "SKIP route_simulation_over_the_local_stand: set SOLANA_RPC_URL={}",
            manifest.rpc
        );
        return;
    }
    let route_so = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/deploy/titan_v3_venue_template.so");
    assert!(route_so.exists(), "run make build-program first");

    let mut rows = Vec::new();
    let mut failures = 0;
    for pool in manifest.pools.iter().filter(|p| !p.dynamic) {
        let address: Pubkey = pool.address.parse().unwrap();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            // The venue declares every ordered pair (structural); the route
            // harness unwraps `bounds` per direction, so it sees the venue
            // through `QuotableDirections`: a deactivated input token or a
            // switched-off pool declares directions that have no quotable
            // range and are asserted by the matrix tests instead.
            rt.block_on(run_swap_route::<QuotableDirections<CofferVenue>>(
                RouteConfig {
                    pool: address,
                    venue_programs: vec![COFFER_PROGRAM_ID],
                },
            ))
        });
        let status = match tokio::task::spawn_blocking(move || handle.join())
            .await
            .unwrap()
        {
            Ok(()) => "ok".to_string(),
            Err(payload) => {
                failures += 1;
                let msg = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                format!("FAIL: {}", msg.lines().next().unwrap_or(""))
            }
        };
        rows.push((pool.case.clone(), status));
    }
    println!("\n== route simulation (swap_route_v3 in LiteSVM) over the local stand ==");
    for (case, status) in &rows {
        println!("  {case:<20} {status}");
    }
    assert_eq!(
        failures, 0,
        "{failures} pool(s) failed the route simulation"
    );
}

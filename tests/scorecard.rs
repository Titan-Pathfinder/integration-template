//! Quay integration scorecard — an always-on, no-RPC report.
//!
//! Prints the four integration layers, any remaining `FILL_IN:` markers, and
//! whether the on-chain simulation prerequisites are present.
//!
//! cargo hides passing-test output unless `--nocapture`. To see the report:
//!
//! ```bash
//! make scorecard
//! cargo test --release --test scorecard -- --nocapture
//! ```

use std::fs;
use std::path::{Path, PathBuf};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Read a repo-relative file, returning "" if it does not exist.
fn read(rel: &str) -> String {
    fs::read_to_string(manifest().join(rel)).unwrap_or_default()
}

/// Every file under a repo-relative directory, recursively.
fn files_under(rel: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&manifest().join(rel), &mut out);
    out
}

/// Files (with counts) that still contain actionable `FILL_IN:` markers, sorted
/// most-first. Matches the colon form so prose mentions don't count, and skips
/// this scorecard file (which references the marker in its own logic).
fn fill_in_files() -> Vec<(String, usize)> {
    let root = manifest();
    let mut paths: Vec<PathBuf> = ["src", "tests", "program-template/programs"]
        .iter()
        .flat_map(|&r| files_under(r))
        .collect();
    paths.push(root.join("program-template/Anchor.toml"));

    let mut out: Vec<(String, usize)> = paths
        .into_iter()
        .filter(|p| !p.ends_with("scorecard.rs"))
        .filter_map(|p| {
            let n = fs::read_to_string(&p).ok()?.matches("FILL_IN:").count();
            (n > 0).then(|| {
                let rel = p
                    .strip_prefix(&root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .into_owned();
                (rel, n)
            })
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Whether the Quay program binary the simulation tests need is dumped.
fn programs_present() -> bool {
    manifest()
        .join("programs")
        .join("QUayE6nexQWYNZAEqfN8FxoNwQDSu3CAzT2qq9J1ArG.so")
        .exists()
}

/// The integration layers, with their detail strings.
const LAYERS: [(&str, &str); 4] = [
    ("Creation parser", "parse_pool_creations() + a fixture test"),
    (
        "Quote layer",
        "implements quote() returning output + a marginal price",
    ),
    ("Program layer", "on-chain CPI module + a Venue enum variant"),
    (
        "Route builder",
        "protocol_to_venue mapping + a Venue enum variant",
    ),
];

fn render_subheader(title: &str) -> String {
    let width = 62usize;
    let prefix = format!("  -- {title} ");
    let dashes = "-".repeat(width.saturating_sub(prefix.len()));
    format!("{prefix}{dashes}\n")
}

fn render_layers(done: [bool; 4]) -> String {
    let mut s = String::from("  Quay venue:\n\n");
    s.push_str(&render_subheader("Layers"));
    s.push_str("  Status  Layer            Detail\n");
    s.push_str(
        "  ------  ---------------  ------------------------------------------------------------\n",
    );
    for (i, (layer, desc)) in LAYERS.iter().enumerate() {
        s.push_str(&format!(
            "  {:<6}  {:<15}  {}\n",
            if done[i] { "[x]" } else { "[ ]" },
            layer,
            desc
        ));
    }
    s
}

fn render_fill_in(fill_in: &[(String, usize)]) -> String {
    let fill_in_total: usize = fill_in.iter().map(|(_, n)| n).sum();
    let mut s = String::new();
    s.push('\n');
    s.push_str(&render_subheader("Remaining FILL_IN markers"));
    s.push_str(&format!(
        "  Total: {fill_in_total} marker(s) across {} file(s)\n",
        fill_in.len()
    ));
    s.push_str("  Count  File\n");
    s.push_str("  -----  ------------------------------------------------------------\n");
    for (path, n) in fill_in {
        s.push_str(&format!("  {n:<5}  {path}\n"));
    }
    s
}

fn render_simulation() -> String {
    let (status, detail) = if std::env::var("SOLANA_RPC_URL").is_ok() && programs_present() {
        ("ENABLED", "SOLANA_RPC_URL set and the Quay program dump present")
    } else {
        ("SKIPPED", "set SOLANA_RPC_URL and run `make dump-programs`")
    };

    format!(
        "\n{}  Status    Detail\n  --------  ------------------------------------------------------------\n  {status:<8}  {detail}\n",
        render_subheader("Simulation")
    )
}

fn render_summary(done: [bool; 4]) -> String {
    let count = done.iter().filter(|d| **d).count();
    let detail = if done.iter().all(|d| *d) {
        "all wired, run the sim tests to validate"
    } else {
        "replace the [ ] items above"
    };

    format!(
        "\n{}  Target      Status             Detail\n  ----------  -----------------  ------------------------------------------------------------\n  {:<10}  {count}/4 layers wired  {detail}\n",
        render_subheader("Summary"),
        "Quay venue"
    )
}

#[test]
fn integration_scorecard() {
    const PROGRAM_SRC: &str = "program-template/programs/titan-v3-venue-template/src";

    // Structural integrity: every layer file must exist.
    for f in [
        "src/quay/mod.rs",
        "src/swap_route/mod.rs",
        "tests/quay_creation.rs",
        &format!("{PROGRAM_SRC}/state.rs"),
        &format!("{PROGRAM_SRC}/instructions/venues/quay.rs"),
    ] {
        assert!(!read(f).is_empty(), "integration layer missing or empty: {f}");
    }

    let quay = read("src/quay/mod.rs");
    let quay_cpi = read(&format!("{PROGRAM_SRC}/instructions/venues/quay.rs"));
    let swap_route = read("src/swap_route/mod.rs");
    let state = read(&format!("{PROGRAM_SRC}/state.rs"));
    let quay_creation = read("tests/quay_creation.rs");

    let done = [
        quay.contains("fn parse_pool_creations")
            && quay_creation.contains("parses_quay_pool_creation"),
        quay.contains("fn quote") && quay.contains("price"),
        !quay_cpi.is_empty() && state.contains("Venue") && state.contains("Quay"),
        swap_route.contains("PoolProtocol::Quay") && swap_route.contains("Venue::Quay"),
    ];

    let mut report = String::new();
    report.push_str("\n================ Quay integration scorecard =================\n\n");
    report.push_str(&render_layers(done));
    report.push_str(&render_fill_in(&fill_in_files()));
    report.push_str(&render_simulation());
    report.push_str(&render_summary(done));
    report.push_str("=============================================================\n");
    println!("{report}");

    assert!(
        done.iter().all(|d| *d),
        "a Quay integration layer is not wired — see the [ ] rows above",
    );
}

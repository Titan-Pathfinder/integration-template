//! Integration scorecard — an always-on, no-RPC report.
//!
//! The same layer-checkboxes are evaluated
//! twice: once for the worked **Example** (Raydium — should be all green) and
//! once for **Your venue**. Which section prints is controlled by the
//! `SCORECARD_SECTION` env var (`example`, `venue`, or `both`; default `both`),
//! which the Makefile sets per target.
//!
//! cargo hides passing-test output unless `--nocapture`. To see the report:
//!
//! ```bash
//! make scorecard                                   # both sections
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

/// Whether the venue program binaries the simulation tests need are dumped.
fn programs_present() -> bool {
    [
        "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8.so",
        "sspUE1vrh7xRoXxGsg7vR1zde2WdGtJRbyK9uRumBDy.so",
        "ssmbu3KZxgonUtjEMCKspZzxvUQCxAFnyh1rcHUeEDo.so",
    ]
    .iter()
    .all(|p| manifest().join("programs").join(p).exists())
}

/// The integration layers, with identical labels for both sections.
const LAYERS: [(&str, &str); 4] = [
    ("Creation parser", "parse_pool_creations() + a fixture test"),
    (
        "Quote layer",
        "implements quote() returning output + a marginal price",
    ),
    (
        "Program layer",
        "on-chain CPI module + a Venue enum variant",
    ),
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

fn render_layers(title: &str, done: [bool; 4]) -> String {
    let mut s = format!("  {title}\n\n");
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
        ("ENABLED", "SOLANA_RPC_URL set and program dumps present")
    } else {
        ("SKIPPED", "set SOLANA_RPC_URL and run `make dump-programs`")
    };

    format!(
        "\n{}  Status    Detail\n  --------  ------------------------------------------------------------\n  {status:<8}  {detail}\n",
        render_subheader("Simulation")
    )
}

fn render_summary(title: &str, done: [bool; 4]) -> String {
    let count = done.iter().filter(|d| **d).count();
    let detail = if done.iter().all(|d| *d) {
        "all wired, run the sim tests to validate"
    } else {
        "replace the [ ] items above"
    };

    format!(
        "\n{}  Target      Status             Detail\n  ----------  -----------------  ------------------------------------------------------------\n  {title:<10}  {count}/4 layers wired  {detail}\n",
        render_subheader("Summary")
    )
}

#[test]
fn integration_scorecard() {
    const PROGRAM_SRC: &str = "program-template/programs/titan-v3-venue-template/src";

    // Structural integrity: every layer file must exist.
    for f in [
        "src/example/mod.rs",
        "src/your_venue/mod.rs",
        "src/swap_route/mod.rs",
        "tests/venue_creation.rs",
        "tests/your_venue_creation.rs",
        &format!("{PROGRAM_SRC}/state.rs"),
    ] {
        assert!(
            !read(f).is_empty(),
            "integration layer missing or empty: {f}"
        );
    }

    let example = read("src/example/mod.rs");
    let raydium_cpi = read(&format!("{PROGRAM_SRC}/instructions/venues/raydium_amm.rs"));
    let swap_route = read("src/swap_route/mod.rs");
    let state = read(&format!("{PROGRAM_SRC}/state.rs"));
    let template_venue = read(&format!("{PROGRAM_SRC}/instructions/venues/template.rs"));
    let your_venue = read("src/your_venue/mod.rs");
    let venue_creation = read("tests/venue_creation.rs");
    let your_venue_creation = read("tests/your_venue_creation.rs");

    // Same layers, evaluated for the reference example...
    let example_done = [
        example.contains("fn parse_pool_creations")
            && venue_creation.contains("parses_raydium_pool_creation"),
        example.contains("fn quote") && example.contains("fn price"),
        !raydium_cpi.is_empty() && state.contains("RaydiumAmm"),
        swap_route.contains("PoolProtocol::RaydiumAMM") && swap_route.contains("Venue::RaydiumAmm"),
    ];
    // ...and for your venue (placeholders replaced / stub implemented).
    let venue_done = [
        !your_venue.contains("YourVenue::parse_pool_creations")
            && !your_venue_creation.contains("FILL_IN:")
            && !your_venue_creation.contains("todo!("),
        !your_venue.contains("todo!("),
        !state.contains("TemplateVenue")
            && !template_venue.contains("11111111111111111111111111111111"),
        !swap_route.contains("TemplateVenue"),
    ];

    let section = std::env::var("SCORECARD_SECTION").unwrap_or_else(|_| "both".into());
    let show_example = section != "venue";
    let show_venue = section != "example";

    let mut report = String::new();
    report.push_str("\n================ Titan integration scorecard ================\n\n");

    if show_example {
        report.push_str(&render_layers(
            "Example (reference — should be all green):",
            example_done,
        ));
        report.push_str(&render_summary("Example", example_done));
        if show_venue {
            report.push('\n');
        }
    }

    if show_venue {
        report.push_str(&render_layers("Your venue (fill these in):", venue_done));

        let fill_in = fill_in_files();
        report.push_str(&render_fill_in(&fill_in));
        report.push_str(&render_simulation());
        report.push_str(&render_summary("Your venue", venue_done));
    }

    report.push_str("=============================================================\n");
    println!("{report}");

    // The reference example is a regression guard: it must always be complete.
    assert!(
        example_done.iter().all(|d| *d),
        "the reference (example) integration is incomplete or broken — its layers \
         should all be wired",
    );
}

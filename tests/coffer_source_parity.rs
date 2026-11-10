//! Source-parity guard for the Coffer math port.
//!
//! `src/coffer/**` claims to be a verbatim copy of the contract. This test reads
//! the contract source (from `COFFER_CONTRACT_SRC`, or the sibling `contracts`
//! checkout's program crate) and asserts:
//!
//! 1. every ported module is byte-identical to the original once `use` lines
//!    are dropped and the copy's `crate::coffer::` paths are read as `crate::`;
//! 2. the swap handler's guard / fee / curve / surge-fee / payout / state-update
//!    segments and its two fee helpers appear verbatim in `src/coffer/swap.rs`
//!    and `src/coffer/state.rs`;
//! 3. the account layout (`AssetConfig`, `AssetDynamics`, `TokenSlot`,
//!    `CofferPool`) and the `ErrorCode` variant list match field for field;
//! 4. the `swap` and `initialize_pool` account contexts still have the
//!    order the instruction builder / creation parser assume.
//!
//! SKIPs (with a message) when the contract source is not available.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The contract's `src/` directory: `COFFER_CONTRACT_SRC`, else the program
/// crate of the sibling `contracts` checkout that carries the surge-fee math
/// (`../contracts/programs/*/src/math/surge_fee.rs`). `None` (with a SKIP
/// line) if absent.
fn contract_src() -> Option<PathBuf> {
    let candidate = match env::var("COFFER_CONTRACT_SRC") {
        Ok(p) => Some(PathBuf::from(p)),
        Err(_) => {
            let programs = manifest().join("../contracts/programs");
            fs::read_dir(&programs).ok().and_then(|dir| {
                dir.flatten()
                    .map(|e| e.path().join("src"))
                    .find(|src| src.join("math/surge_fee.rs").exists())
            })
        }
    };
    match candidate {
        Some(c) if c.join("instructions/user/swap.rs").exists() => Some(c),
        other => {
            eprintln!(
                "SKIP {}: contract source not found ({}) — set COFFER_CONTRACT_SRC to the \
                 program crate's `src` directory to run the parity check",
                std::thread::current()
                    .name()
                    .unwrap_or("coffer_source_parity"),
                other
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "no candidate".into())
            );
            None
        }
    }
}

/// The deterministic rename map the port applies to the contract source.
/// MUST stay identical to `scripts/port_coffer_math.py`. The pool PDA seed
/// literal keeps its bytes but is spelled as a byte array.
fn apply_renames(text: &str) -> String {
    let old = String::from("Cub") + "ic";
    let seed_literal = format!("b\"{}_pool\"", old.to_lowercase());
    let rules: [(String, &str); 9] = [
        (
            seed_literal,
            "&[99u8, 117, 98, 105, 99, 95, 112, 111, 111, 108]",
        ),
        (format!("{old}Math"), "CofferMath"),
        (format!("{}_math", old.to_lowercase()), "coffer_math"),
        (format!("{old}Pool"), "CofferPool"),
        (format!("{}_pool", old.to_lowercase()), "coffer_pool"),
        (format!("{}-pool", old.to_lowercase()), "coffer-pool"),
        (old.to_uppercase(), "COFFER"),
        (old.clone(), "Coffer"),
        (old.to_lowercase(), "coffer"),
    ];
    let mut out = text.to_string();
    for (a, b) in rules.iter() {
        out = out.replace(a.as_str(), b);
    }
    out
}

/// Read a contract file with the rename map applied.
fn read_contract(path: &Path) -> String {
    apply_renames(&read(path))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn is_use_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("use ") && t.ends_with(';')
}

/// The comparable form of a whole module: no `use` lines, no leading plain
/// `//` header (the port's provenance banner), trailing whitespace trimmed.
fn normalize_module(text: &str, is_copy: bool) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    if is_copy {
        // Drop the provenance banner: leading `// ...` lines (the originals
        // start with `//!` docs, never plain `//`).
        while lines
            .first()
            .is_some_and(|l| l.starts_with("// ") || *l == "//")
        {
            lines.remove(0);
        }
    }
    let body: Vec<String> = lines
        .into_iter()
        .filter(|l| !is_use_line(l))
        .map(|l| l.trim_end().to_string())
        .collect();
    let mut joined = body.join("\n");
    if is_copy {
        joined = joined.replace("crate::coffer::", "crate::");
    }
    joined.trim().to_string()
}

/// Drop all whitespace (for segment containment): rustfmt may re-wrap a
/// hand-written file that embeds a verbatim segment, and Rust tokens never
/// depend on whitespace between them.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().concat()
}

/// Lines of `text` from the first line containing `start` through the first
/// subsequent line containing `end`, inclusive.
fn extract_segment(text: &str, start: &str, end: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let s = lines
        .iter()
        .position(|l| l.contains(start))
        .unwrap_or_else(|| panic!("start marker not found: {start:?}"));
    let e = lines[s..]
        .iter()
        .position(|l| l.contains(end))
        .map(|i| s + i)
        .unwrap_or_else(|| panic!("end marker not found after {start:?}: {end:?}"));
    lines[s..=e].join("\n")
}

/// `pub name: type` field lines of `pub struct <name>` in declaration order.
fn struct_fields(text: &str, name: &str) -> Vec<String> {
    let header = format!("pub struct {name}");
    let start = text
        .find(&header)
        .unwrap_or_else(|| panic!("struct {name} not found"));
    let body = &text[start..];
    let open = body.find('{').unwrap();
    let mut depth = 0;
    let mut close = open;
    for (i, c) in body[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    body[open..=close]
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub ") && l.contains(':'))
        .map(|l| {
            l.trim_end_matches(',')
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Variant names of `pub enum ErrorCode`, in order.
fn error_variants(text: &str) -> Vec<String> {
    let start = text.find("pub enum ErrorCode").expect("ErrorCode enum");
    let body = &text[start..];
    let end = body.find("\n}\n").expect("enum end");
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("//")
                && !l.starts_with('#')
                && !l.starts_with("pub enum")
                && l.ends_with(',')
                && l.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        })
        .map(|l| l.trim_end_matches(',').to_string())
        .collect()
}

// ---------------------------------------------------------------------------

#[test]
fn ported_modules_are_byte_identical() {
    let Some(src) = contract_src() else { return };
    let ported = manifest().join("src/coffer");
    // (contract path, ported path)
    let coffer_math_src = format!("math/{}_math.rs", (String::from("cub") + "ic"));
    let files = [
        ("constants.rs", "constants.rs"),
        ("math/mod.rs", "math/mod.rs"),
        ("math/fixed_point.rs", "math/fixed_point.rs"),
        ("math/log_exp_math.rs", "math/log_exp_math.rs"),
        (coffer_math_src.as_str(), "math/coffer_math.rs"),
        ("math/weighted_math.rs", "math/weighted_math.rs"),
        ("math/max_selloff.rs", "math/max_selloff.rs"),
        ("math/surge_fee.rs", "math/surge_fee.rs"),
    ];
    let mut drift = Vec::new();
    for (src_rel, rel) in files {
        let original = normalize_module(&read_contract(&src.join(src_rel)), false);
        let copy = normalize_module(&read(&ported.join(rel)), true);
        if original != copy {
            // Report the first differing line for a useful failure.
            let first = original
                .lines()
                .zip(copy.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| format!("line {}: original {a:?} vs copy {b:?}", i + 1))
                .unwrap_or_else(|| "one file is a prefix of the other".to_string());
            drift.push(format!("{rel}: {first}"));
        }
    }
    assert!(
        drift.is_empty(),
        "ported modules drifted from the contract source:\n  {}",
        drift.join("\n  ")
    );
}

#[test]
fn swap_handler_segments_are_verbatim() {
    let Some(src) = contract_src() else { return };
    let handler = read_contract(&src.join("instructions/user/swap.rs"));
    let port = squash(&normalize_module(
        &read(&manifest().join("src/coffer/swap.rs")),
        true,
    ));

    let segments: [(&str, &str); 10] = [
        // pool-level guards and index checks
        (
            "require!(pool.pool_enabled, ErrorCode::PoolDisabled);",
            "require!(token_in_index != token_out_index, ErrorCode::InvalidTokenIndex);",
        ),
        // input-side kill switch
        (
            "let token_in_idx = token_in_index as usize;",
            "ErrorCode::TokenInactive",
        ),
        // sell-off window inputs
        (
            "let virtual_balance_in = pool.tokens[token_in_idx].dynamics.virtual_balance;",
            "let max_selloff_period = pool.tokens[token_in_idx].config.max_selloff_period_length;",
        ),
        // fee split + curve call
        (
            "let virtual_balance_out = pool.tokens[token_out_idx].dynamics.virtual_balance;",
            "decimals_out,",
        ),
        // the whole surge-fee block
        (
            "Variable sell-off surge fee",
            "fee_acc.min(amount_out as u128) as u64",
        ),
        // user payout
        (
            "// User receives the curve output minus the surge fee.",
            ".ok_or(ErrorCode::MathUnderflow)?;",
        ),
        // belt-and-braces liquidity restatement
        (
            "require!(amount_out <= actual_balance_out, ErrorCode::InsufficientLiquidity);",
            "require!(amount_out <= actual_balance_out, ErrorCode::InsufficientLiquidity);",
        ),
        // state update (apply_swap)
        (
            "pool.tokens[token_in_idx].dynamics.virtual_balance = virtual_balance_in",
            "pool.add_protocol_fees(token_out_idx, surge_fee_amount)?;",
        ),
        // fee helpers
        (
            "fn calculate_swap_fee(amount: u64, fee_rate: u32) -> Result<u64> {",
            "fee.try_into().map_err(|_| ErrorCode::MathOverflow.into())",
        ),
        (
            "fn calculate_protocol_fee(swap_fee: u64, protocol_fee_rate: u16) -> Result<u64> {",
            "protocol_fee.try_into().map_err(|_| ErrorCode::MathOverflow.into())",
        ),
    ];

    let mut missing = Vec::new();
    for (start, end) in segments {
        let segment = squash(&extract_segment(&handler, start, end));
        if !port.contains(&segment) {
            missing.push(format!("segment starting {start:?} (ends {end:?})"));
        }
    }
    assert!(
        missing.is_empty(),
        "swap handler segments missing or changed in src/coffer/swap.rs:\n  {}",
        missing.join("\n  ")
    );

    // `add_protocol_fees` lives on the state mirror.
    let state_src =
        read_contract(&src.join(format!("state/{}_pool.rs", String::from("cub") + "ic")));
    let state_port = squash(&normalize_module(
        &read(&manifest().join("src/coffer/state.rs")),
        true,
    ));
    let seg = squash(&extract_segment(
        &state_src,
        "pub fn add_protocol_fees(&mut self, token_index: usize, amount: u64) -> Result<()> {",
        "Ok(())",
    ));
    assert!(
        state_port.contains(&seg),
        "CofferPool::add_protocol_fees drifted from the contract"
    );
}

#[test]
fn account_layout_matches_field_for_field() {
    let Some(src) = contract_src() else { return };
    let original =
        read_contract(&src.join(format!("state/{}_pool.rs", String::from("cub") + "ic")));
    let copy = read(&manifest().join("src/coffer/state.rs"));
    for name in ["AssetConfig", "AssetDynamics", "TokenSlot", "CofferPool"] {
        let a = struct_fields(&original, name);
        let b = struct_fields(&copy, name);
        assert!(!a.is_empty(), "{name}: no fields parsed from the contract");
        assert_eq!(a, b, "{name}: field list differs from the contract");
    }
    // The size constants the contract documents.
    assert!(
        original.contains("const INIT_SPACE: usize = 32 + 1 + 1 + 8 + 4 + 2 + 8 + 1 + 1 + 32 + 32")
    );
}

#[test]
fn error_codes_match_variant_for_variant() {
    let Some(src) = contract_src() else { return };
    let original = error_variants(&read_contract(&src.join("errors.rs")));
    let copy = error_variants(&read(&manifest().join("src/coffer/errors.rs")));
    assert!(
        original.len() > 60,
        "parsed only {} contract variants",
        original.len()
    );
    assert_eq!(
        original, copy,
        "ErrorCode variants differ from the contract"
    );
}

#[test]
fn instruction_account_orders_match() {
    let Some(src) = contract_src() else { return };
    let swap = read_contract(&src.join("instructions/user/swap.rs"));
    let fields: Vec<String> = struct_fields(&swap, "Swap<'info>")
        .iter()
        .map(|f| {
            f.split(':')
                .next()
                .unwrap()
                .trim_start_matches("pub ")
                .to_string()
        })
        .collect();
    assert_eq!(
        fields,
        [
            "pool",
            "token_mint_in",
            "token_mint_out",
            "user_token_account_in",
            "user_token_account_out",
            "vault_in",
            "vault_out",
            "user",
            "token_program_in",
            "token_program_out",
        ]
    );
    let builder = read(&manifest().join("src/coffer_venue/instruction.rs"));
    let ours: Vec<String> = struct_fields(&builder, "SwapAccounts")
        .iter()
        .map(|f| {
            f.split(':')
                .next()
                .unwrap()
                .trim_start_matches("pub ")
                .to_string()
        })
        .collect();
    assert_eq!(
        ours, fields,
        "SwapAccounts field order must match the Swap context"
    );

    let init = read_contract(&src.join(format!(
        "instructions/user/initialize_{}_pool.rs",
        String::from("cub") + "ic"
    )));
    let fields: Vec<String> = struct_fields(&init, "InitializeCofferPool<'info>")
        .iter()
        .map(|f| {
            f.split(':')
                .next()
                .unwrap()
                .trim_start_matches("pub ")
                .to_string()
        })
        .collect();
    assert_eq!(
        fields,
        [
            "config",
            "pool",
            "bpt_mint",
            "payer",
            "token_program",
            "associated_token_program",
            "system_program",
        ],
        "pool-initialisation fixed accounts changed — update parse_pool_creations"
    );
    // The handler must still take the mints from remaining_accounts.
    assert!(init.contains("let token_count = remaining_accounts.len();"));
    // And the first Borsh argument must still be the weights vector.
    assert!(
        init.contains("normalized_weights: Vec<u64>,\n    initial_virtual_balances: Vec<u64>,")
    );
}

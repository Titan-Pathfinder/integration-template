// Quoting parity + speed tests for Dr. Fraudsworth TradingVenue integration.

use solana_sdk::pubkey::Pubkey;
use titan_integration_template::trading_venue::{QuoteRequest, SwapType, TradingVenue};
use titan_integration_template::drfraudsworth::accounts::addresses::*;
use titan_integration_template::drfraudsworth::constants::*;
use titan_integration_template::drfraudsworth::math::amm_math::*;
use titan_integration_template::drfraudsworth::math::tax_math::*;
use titan_integration_template::drfraudsworth::math::vault_math::*;
use titan_integration_template::drfraudsworth::sol_pool_venue::SolPoolVenue;
use titan_integration_template::drfraudsworth::vault_venue::VaultVenue;

// Reference pipeline (replicates on-chain math)
fn ref_buy(rsol: u64, rtok: u64, amt: u64, tax: u16, fee: u16) -> u64 {
    if amt == 0 { return 0; }
    let t = calculate_tax(amt, tax).unwrap();
    let s = amt.checked_sub(t).unwrap();
    if s == 0 { return 0; }
    let e = calculate_effective_input(s, fee).unwrap();
    calculate_swap_output(rsol, rtok, e).unwrap_or(0)
}
fn ref_sell(rsol: u64, rtok: u64, amt: u64, tax: u16, fee: u16) -> u64 {
    if amt == 0 { return 0; }
    let e = calculate_effective_input(amt, fee).unwrap();
    let g = calculate_swap_output(rtok, rsol, e).unwrap_or(0);
    g.saturating_sub(calculate_tax(g, tax).unwrap())
}

const RS: u64 = 100_000_000_000;
const RT: u64 = 1_000_000_000_000;
const BT: u16 = 400;
const ST: u16 = 1400;

fn crime() -> SolPoolVenue { SolPoolVenue::new_for_testing(true, RS, RT, BT, ST) }

// --- Parity ---
#[test]
fn parity_buy_1sol() {
    let v = crime(); let a = 1_000_000_000;
    let r = v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: a, swap_type: SwapType::ExactIn }).unwrap();
    assert_eq!(r.expected_output, ref_buy(RS, RT, a, BT, LP_FEE_BPS));
}

#[test]
fn parity_sell_1m() {
    let v = crime(); let a = 1_000_000;
    let r = v.quote(QuoteRequest { input_mint: CRIME_MINT, output_mint: NATIVE_MINT, amount: a, swap_type: SwapType::ExactIn }).unwrap();
    assert_eq!(r.expected_output, ref_sell(RS, RT, a, ST, LP_FEE_BPS));
}

#[test]
fn parity_vault_all_directions() {
    for (i, o, a) in [(CRIME_MINT, PROFIT_MINT, 10_000u64), (FRAUD_MINT, PROFIT_MINT, 10_000), (PROFIT_MINT, CRIME_MINT, 100), (PROFIT_MINT, FRAUD_MINT, 100)] {
        let v = VaultVenue::new_for_testing(i, o);
        let r = v.quote(QuoteRequest { input_mint: i, output_mint: o, amount: a, swap_type: SwapType::ExactIn }).unwrap();
        assert_eq!(r.expected_output, compute_vault_output(&i, &o, a).unwrap());
    }
}

// --- Random sampling (50 per direction) ---
fn sample(lo: u64, hi: u64, seed: u64) -> u64 {
    let h = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let f = (h as f64) / (u64::MAX as f64);
    ((lo as f64).ln() + f * ((hi as f64).ln() - (lo as f64).ln())).exp().max(lo as f64).min(hi as f64) as u64
}

#[test]
fn random_buy_50() {
    let v = crime();
    for s in 0..50 {
        let a = sample(1_000, 50_000_000_000, s);
        let r = v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: a, swap_type: SwapType::ExactIn }).unwrap();
        assert_eq!(r.expected_output, ref_buy(RS, RT, a, BT, LP_FEE_BPS), "sample {} at {}", s, a);
    }
}

#[test]
fn random_sell_50() {
    let v = crime();
    for s in 0..50 {
        let a = sample(1_000, 500_000_000_000, s + 100);
        let r = v.quote(QuoteRequest { input_mint: CRIME_MINT, output_mint: NATIVE_MINT, amount: a, swap_type: SwapType::ExactIn }).unwrap();
        assert_eq!(r.expected_output, ref_sell(RS, RT, a, ST, LP_FEE_BPS), "sample {} at {}", s, a);
    }
}

// --- Monotonicity ---
#[test]
fn monotone_buy_100() {
    let v = crime(); let mut prev = 0;
    for i in 1..=100 {
        let r = v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: i * 500_000_000, swap_type: SwapType::ExactIn }).unwrap();
        assert!(r.expected_output >= prev); prev = r.expected_output;
    }
}

#[test]
fn monotone_sell_100() {
    let v = crime(); let mut prev = 0;
    for i in 1..=100 {
        let r = v.quote(QuoteRequest { input_mint: CRIME_MINT, output_mint: NATIVE_MINT, amount: i * 5_000_000_000, swap_type: SwapType::ExactIn }).unwrap();
        assert!(r.expected_output >= prev); prev = r.expected_output;
    }
}

// --- Speed ---
#[test]
fn speed_buy_10k() {
    let v = crime();
    let s = std::time::Instant::now();
    for i in 0..10_000u64 { let _ = v.quote(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: 1_000_000 + i, swap_type: SwapType::ExactIn }); }
    let avg = s.elapsed().as_micros() as f64 / 10_000.0;
    assert!(avg < 100.0, "avg {:.2}us > 100us", avg);
}

#[test]
fn speed_sell_10k() {
    let v = crime();
    let s = std::time::Instant::now();
    for i in 0..10_000u64 { let _ = v.quote(QuoteRequest { input_mint: CRIME_MINT, output_mint: NATIVE_MINT, amount: 1_000_000 + i, swap_type: SwapType::ExactIn }); }
    assert!(s.elapsed().as_micros() as f64 / 10_000.0 < 100.0);
}

#[test]
fn speed_vault_10k() {
    let v = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
    let s = std::time::Instant::now();
    for i in 0..10_000u64 { let _ = v.quote(QuoteRequest { input_mint: CRIME_MINT, output_mint: PROFIT_MINT, amount: 10_000 + i, swap_type: SwapType::ExactIn }); }
    assert!(s.elapsed().as_micros() as f64 / 10_000.0 < 100.0);
}

// --- Instruction structure ---
#[test]
fn buy_ix_structure() {
    let v = crime();
    let ix = v.generate_swap_instruction(QuoteRequest { input_mint: NATIVE_MINT, output_mint: CRIME_MINT, amount: 1_000_000_000, swap_type: SwapType::ExactIn }, Pubkey::new_unique()).unwrap();
    assert_eq!(ix.program_id, TAX_PROGRAM_ID);
    assert_eq!(ix.accounts.len(), 24);
    assert_eq!(ix.data.len(), 25);
    assert!(ix.accounts[0].is_signer);
}

#[test]
fn sell_ix_structure() {
    let v = crime();
    let ix = v.generate_swap_instruction(QuoteRequest { input_mint: CRIME_MINT, output_mint: NATIVE_MINT, amount: 1_000_000_000, swap_type: SwapType::ExactIn }, Pubkey::new_unique()).unwrap();
    assert_eq!(ix.program_id, TAX_PROGRAM_ID);
    assert_eq!(ix.accounts.len(), 25);
    assert_eq!(ix.data.len(), 25);
}

#[test]
fn vault_ix_structure() {
    let v = VaultVenue::new_for_testing(CRIME_MINT, PROFIT_MINT);
    let ix = v.generate_swap_instruction(QuoteRequest { input_mint: CRIME_MINT, output_mint: PROFIT_MINT, amount: 10_000, swap_type: SwapType::ExactIn }, Pubkey::new_unique()).unwrap();
    assert_eq!(ix.program_id, CONVERSION_VAULT_PROGRAM_ID);
    assert_eq!(ix.accounts.len(), 17);
    assert_eq!(ix.data.len(), 16);
}

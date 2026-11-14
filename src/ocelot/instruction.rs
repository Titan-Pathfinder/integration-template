//! Exact v3 instruction encoding and account order. The runtime must spend the quoted input.
use super::{
    codec::{Result, invalid},
    *,
};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

pub fn ioc(lots: u64, ticks: u64, buy: bool, reduce_only: bool) -> Vec<u8> {
    let mut d = vec![2, u8::from(!buy), 1];
    d.extend(ticks.to_le_bytes());
    d.extend(lots.to_le_bytes());
    d.push(0);
    d.extend(lots.to_le_bytes());
    d.extend(u64::from(reduce_only).to_le_bytes());
    d.extend([0, 0]);
    d.extend([0; 16]);
    d.extend([0, if reduce_only { 128 } else { 0 }, 0]);
    d
}
fn vector(d: &mut Vec<u8>, v: &[u8]) {
    d.extend((v.len() as u32).to_le_bytes());
    d.extend(v);
}
// Mirrors the deployed instruction fields; keep the wire order explicit.
#[allow(clippy::too_many_arguments)]
pub fn encode(
    mint: bool,
    amount: u64,
    net: u64,
    order: &[u8],
    counts: [u8; 3],
    min_output: u64,
    deadline: i64,
    revision: u64,
) -> Vec<u8> {
    let mut d = discriminator(if mint {
        "global:phoenix_mint_v3"
    } else {
        "global:phoenix_redeem_v3"
    })
    .to_vec();
    d.extend(amount.to_le_bytes());
    if mint {
        for _ in 0..3 {
            vector(&mut d, &net.to_le_bytes());
        }
        vector(&mut d, order);
        d.extend(counts);
        d.extend(0u64.to_le_bytes());
    } else {
        vector(&mut d, order);
        d.extend(1u64.to_le_bytes());
        d.extend(counts);
    }
    d.extend(min_output.to_le_bytes());
    d.extend(deadline.to_le_bytes());
    d.extend(revision.to_le_bytes());
    d
}

fn ro(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}
fn rw(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
pub fn pda(seed: &[u8], market: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seed, market.as_ref()], &PROGRAM_ID).0
}
pub fn ata(owner: Pubkey, mint: Pubkey) -> Pubkey {
    spl_associated_token_account::get_associated_token_address_with_program_id(
        &owner,
        &mint,
        &spl_token::ID,
    )
}

#[derive(Clone, Debug)]
pub struct Exchange {
    pub vault: Pubkey,
    pub indices: Vec<Pubkey>,
    pub buffers: Vec<Pubkey>,
    pub withdraw_queue: Pubkey,
}
/// Low-level encoding: a zero minimum requires an enclosing transaction's output
/// check. Standalone callers should use `OcelotVenue::build_with_minimum`.
pub fn build(
    v: &OcelotVenue,
    amount: u64,
    mint: bool,
    user: Pubkey,
    min_output: u64,
) -> Result<Instruction> {
    let s = v.state.as_ref().ok_or(
        crate::trading_venue::error::TradingVenueError::NotInitialized("Ocelot state".into()),
    )?;
    let m = &s.market;
    let c = &s.config;
    let ex = &s.exchange;
    let market = v.pool_id;
    if amount == 0 {
        return Err(invalid("zero input"));
    }
    let usdc_vault = pda(b"phoenix_usdc_vault", market);
    let collateral_vault = pda(b"phoenix_collateral_vault", market);
    let config = pda(b"phoenix", market);
    let spline = Pubkey::find_program_address(&[b"spline", c.book.as_ref()], &PHOENIX).0;
    let ember_state = Pubkey::find_program_address(&[PHOENIX.as_ref(), b"state"], &EMBER).0;
    let ember_vault = Pubkey::find_program_address(&[PHOENIX.as_ref(), b"vault"], &EMBER).0;
    let sizing = super::quote::order(s, amount, mint)?;
    let net = sizing.deposit;
    let order = if !mint && s.position.base == 0 {
        vec![]
    } else {
        ioc(
            sizing.lots,
            sizing.limit_ticks,
            sizing.buy,
            sizing.reduce_only,
        )
    };
    let mut venue_order = vec![
        ro(PHOENIX),
        ro(PHOENIX_LOG),
        rw(PHOENIX_GLOBAL),
        ro(c.authority),
        rw(c.trader),
        rw(PERP_MAP),
    ];
    venue_order.extend(ex.indices.iter().copied().map(rw));
    venue_order.extend(ex.buffers.iter().copied().map(rw));
    venue_order.extend([rw(c.book), rw(spline)]);
    let ember = vec![
        ro(c.authority),
        ro(ember_state),
        ro(m.usdc),
        rw(COLLATERAL),
        rw(usdc_vault),
        rw(collateral_vault),
        rw(ember_vault),
        ro(spl_token::ID),
    ];
    let mut accounts;
    let counts;
    if mint {
        accounts = vec![
            rw(market),
            ro(config),
            rw(m.mint),
            ro(m.usdc),
            ro(COLLATERAL),
            AccountMeta::new(user, true),
            ro(c.authority),
            rw(ata(user, m.usdc)),
            rw(m.fees),
            rw(usdc_vault),
            rw(collateral_vault),
            ro(c.parent),
            ro(c.trader),
            ro(PERP_MAP),
            rw(ata(user, m.mint)),
            ro(spl_token::ID),
            ro(spl_associated_token_account::ID),
            ro(solana_sdk_ids::system_program::ID),
            ro(solana_sdk_ids::sysvar::rent::ID),
            ro(EMBER),
            rw(pda(b"finance_v1", market)),
            rw(pda(b"market_cash_v1", market)),
        ];
        let mut deposit = vec![
            ro(PHOENIX),
            ro(PHOENIX_LOG),
            rw(PHOENIX_GLOBAL),
            ro(c.authority),
            rw(collateral_vault),
            rw(c.parent),
            rw(ex.vault),
            ro(spl_token::ID),
        ];
        deposit.extend(ex.indices.iter().copied().map(rw));
        deposit.extend(ex.buffers.iter().copied().map(rw));
        let mut transfer = vec![
            ro(PHOENIX),
            ro(PHOENIX_LOG),
            ro(PHOENIX_GLOBAL),
            ro(c.authority),
            rw(c.parent),
            rw(c.trader),
            ro(PERP_MAP),
        ];
        transfer.extend(ex.indices.iter().copied().map(rw));
        transfer.extend(ex.buffers.iter().copied().map(rw));
        counts = [
            u8::try_from(deposit.len()),
            u8::try_from(transfer.len()),
            u8::try_from(venue_order.len()),
        ];
        accounts.extend(ember);
        accounts.extend(deposit);
        accounts.extend(transfer);
        accounts.extend(venue_order);
    } else {
        accounts = vec![
            rw(market),
            ro(config),
            rw(m.mint),
            ro(m.usdc),
            ro(COLLATERAL),
            AccountMeta::new(user, true),
            rw(ata(user, m.mint)),
            rw(ata(user, m.usdc)),
            rw(m.fees),
            ro(c.authority),
            rw(usdc_vault),
            rw(collateral_vault),
            ro(c.trader),
            ro(PERP_MAP),
            rw(global_bucket()),
            rw(Pubkey::find_program_address(
                &[b"withdraw_throttle", b"market", market.as_ref()],
                &PROGRAM_ID,
            )
            .0),
            ro(spl_token::ID),
            ro(spl_associated_token_account::ID),
            ro(solana_sdk_ids::system_program::ID),
            ro(solana_sdk_ids::sysvar::rent::ID),
            ro(EMBER),
            rw(pda(b"finance_v1", market)),
            rw(pda(b"market_cash_v1", market)),
        ];
        let mut withdraw = vec![
            ro(PHOENIX),
            ro(PHOENIX_LOG),
            rw(PHOENIX_GLOBAL),
            ro(c.authority),
            rw(c.trader),
            rw(PERP_MAP),
            rw(ex.vault),
            rw(collateral_vault),
            ro(spl_token::ID),
        ];
        withdraw.extend(ex.indices.iter().copied().map(rw));
        withdraw.extend(ex.buffers.iter().copied().map(rw));
        withdraw.push(rw(ex.withdraw_queue));
        counts = [
            u8::try_from(venue_order.len()),
            u8::try_from(withdraw.len()),
            Ok(8),
        ];
        accounts.extend(venue_order);
        accounts.extend(withdraw);
        accounts.extend(ember);
    }
    if accounts.len() >= 255 {
        return Err(invalid("too many CPI accounts"));
    }
    let [a, b, c] = counts;
    let counts = [
        a.map_err(|_| invalid("account count overflow"))?,
        b.map_err(|_| invalid("account count overflow"))?,
        c.map_err(|_| invalid("account count overflow"))?,
    ];
    Ok(Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: encode(
            mint,
            amount,
            net,
            &order,
            counts,
            min_output,
            // The v3 ABI requires a deadline and checks `now <= deadline`.
            // Titan owns transaction expiry; add no adapter-specific cutoff.
            i64::MAX,
            s.revision,
        ),
    })
}

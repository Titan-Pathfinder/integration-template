//! Native conservative spline quotes. Preparation reads only the supplied cache;
//! queries are allocation-free and never invoke RPC or a transaction VM.
//!
//! Account layouts: rise-public 2ce47221ef04a8d5879d39d31e82f9e92021753e.
//! Shared arena layouts: phoenix-eternal-types-current/sdk/src/accounts and
//! sokoban (validated against captured deployed account data).
use super::{
    COLLATERAL, EMBER, PERP_MAP, PHOENIX, PHOENIX_GLOBAL, State, USDC,
    codec::{Position, Result, account, bytes, i64_at, invalid, key, u16_at, u64_at},
    curve::Curve,
    discriminator,
    quote::{Fill, Order, PhoenixQuoteFactory, PhoenixQuoter},
    spline::{SideLimit, SplineDepth},
};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_pubkey::Pubkey;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(d, o)?))
}
fn get(s: &State, k: Pubkey) -> Result<&Account> {
    s.accounts
        .get(&k)
        .ok_or(invalid("missing native Phoenix dependency"))
}
fn owned<'a>(s: &'a State, k: Pubkey, name: &str, len: usize) -> Result<&'a [u8]> {
    account(get(s, k)?, PHOENIX, &discriminator(name), len)
}
fn narrow(n: i128) -> Result<u64> {
    u64::try_from(n).map_err(|_| invalid("negative or overflowing Phoenix balance"))
}

/// A deliberately stricter margin bound: round leverage down to the next tier,
/// and give no collateral credit for positive unrealized PnL or spot assets.
#[derive(Clone, Debug)]
struct RiskMarket {
    mark: u64,
    funding: i64,
    tiers: [(u64, u64); 4],
    fresh: bool,
    oi_room: u64,
}
impl RiskMarket {
    fn margin(&self, base: u64) -> u128 {
        let leverage = self
            .tiers
            .iter()
            .find(|(bound, _)| base <= *bound)
            .map_or(1, |(_, l)| *l);
        (base as u128 * self.mark as u128).div_ceil(leverage as u128)
    }
}

// PriceComponent layout from the pinned SDK. A fresh aggregate mark alone is
// insufficient: Phoenix can penalize soft-stale oracle components and reject
// withdrawals. We do not model those penalties; require fresh, nondivergent
// component quorum and the exact policy exercised by our captured SOL markets.
// The SDK supplies the layout but not the per-action validity evaluator.
fn fresh_price_components(d: &[u8], o: usize, slot: u64) -> Result<bool> {
    let mut policy = Sha256::new();
    policy.update(&d[o + 352..o + 608]); // Per-action price validity rules.
    policy.update(&d[o + 808..o + 813]); // Radius, quorum, hard-stale multipliers.
    let reviewed: [u8; 32] = [
        0xaf, 0xd9, 0x67, 0x55, 0x2f, 0x05, 0xe6, 0x17, 0xf9, 0x0d, 0x8e, 0xb4, 0x93, 0xda, 0xa3,
        0x05, 0xb9, 0xf0, 0x99, 0xc1, 0xef, 0xe8, 0x70, 0x15, 0xa3, 0x94, 0xd9, 0xbb, 0xb1, 0xf8,
        0xb6, 0xdf,
    ];
    if <[u8; 32]>::from(policy.finalize()) != reviewed {
        return Ok(false);
    }
    let minimum = d[o + 810] as usize;
    if !(1..=5).contains(&minimum) {
        return Ok(false);
    }
    for (start, weight, threshold, count) in
        [(32, 112, 120, 5), (168, 248, 256, 5), (264, 280, 288, 1)]
    {
        // Under the pinned validity rules, spot/perp quorum is required even
        // when its contribution to the aggregate mark is zero. Only the book
        // component can be disabled by a zero weight.
        if count == 1 && u64_at(d, o + weight)? == 0 {
            continue;
        }
        let threshold = u64_at(d, o + threshold)?;
        let mut fresh = 0;
        for i in 0..count {
            let observed = u64_at(d, o + start + i * 16)?;
            if u64_at(d, o + start + i * 16 + 8)? > 0
                && observed <= slot
                && slot - observed < threshold
                // The book component is not an oracle response. Divergence is
                // live state, shared by each oracle's spot/perp observations.
                && (count == 1 || d[o + 640 + 40 * i] == 0)
            {
                fresh += 1;
            }
        }
        if fresh < if count == 1 { 1 } else { minimum } {
            return Ok(false);
        }
    }
    Ok(true)
}

fn markets(s: &State) -> Result<HashMap<u32, RiskMarket>> {
    let d = owned(s, PERP_MAP, "account:perp_asset_map", 48)?;
    let count = u32_at(d, 32)? as usize;
    if count > 1024 || d.len() < 48 + count * 1584 {
        return Err(invalid("invalid Phoenix risk map"));
    }
    let mut result = HashMap::new();
    for i in 0..count {
        let o = 64 + i * 1584;
        // Tombstoned and spot slots are not perpetual risk metadata.
        if d[o + 1450] != 0 || d[o + 1567] != 0 {
            continue;
        }
        let asset = u16_at(d, o + 944)? as u32 | ((u16_at(d, o + 948)? as u32) << 16);
        let mut tiers = [(0, 0); 4];
        for (j, tier) in tiers.iter_mut().enumerate() {
            *tier = (u64_at(d, o + 1016 + 24 * j)?, u64_at(d, o + 1024 + 24 * j)?);
            if tier.0 == 0 || tier.1 == 0 {
                return Err(invalid("invalid leverage tier"));
            }
        }
        if tiers
            .windows(2)
            .any(|t| t[0].0 >= t[1].0 || t[0].1 < t[1].1)
            || u64_at(d, o + 1112)? > 10_000
            || u64_at(d, o + 1200)? > 10_000
        {
            return Err(invalid("unsupported Phoenix risk parameters"));
        }
        let observed = u64_at(d, o + 16)?;
        let mark = u64_at(d, o + 24)?
            .checked_mul(u64_at(d, o + 936)?)
            .filter(|v| *v > 0)
            .ok_or(invalid("invalid risk mark"))?;
        // Special commodity/after-hours and finalized-price rules need their
        // own price bounds. Do not apply the regular perpetual model to them.
        let fresh = observed > 0
            && observed <= s.clock.slot
            && s.clock.slot - observed <= 150
            && fresh_price_components(d, o, s.clock.slot)?
            && d[o + 1456] == 0
            && u64_at(d, o + 1440)? == 0;
        if result
            .insert(
                asset,
                RiskMarket {
                    mark,
                    funding: i64_at(d, o + 1312)?,
                    tiers,
                    fresh,
                    oi_room: u64_at(d, o + 1432)?.saturating_sub(u64_at(d, o + 1424)?),
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate risk market"));
        }
    }
    Ok(result)
}

/// Walk reachable nodes, never free-list contents. All pointers and counts are
/// bounded before access, including cycle detection. Expanded arenas fail closed.
fn tree_nodes(d: &[u8], stride: usize) -> Result<Vec<(u32, usize)>> {
    if d.len() < 96 || u16_at(d, 52)? != 1 || u16_at(d, 54)? != 1 {
        return Err(invalid("unsupported Phoenix arena shape"));
    }
    // num_nodes_per_arena is the eventual arena address-space stride; accounts
    // are grown incrementally and need not allocate that many nodes yet.
    let capacity = (u32_at(d, 56)? as usize).min((d.len() - 96) / stride);
    let count = u32_at(d, 48)? as usize;
    if count > capacity {
        return Err(invalid("invalid Phoenix arena capacity"));
    }
    let mut stack = vec![u32_at(d, 80)?];
    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    while let Some(p) = stack.pop() {
        if p == 0 {
            continue;
        }
        if p as usize > capacity || !seen.insert(p) || seen.len() > count {
            return Err(invalid("invalid Phoenix arena pointer"));
        }
        let o = 96 + (p as usize - 1) * stride;
        stack.extend([u32_at(d, o)?, u32_at(d, o + 4)?]);
        nodes.push((p, o));
    }
    if nodes.len() != count {
        return Err(invalid("Phoenix arena count mismatch"));
    }
    Ok(nodes)
}

struct Maker {
    collateral: i128,
    margin: u128,
    base: i64,
    supported: bool,
}
fn makers(s: &State, risks: &HashMap<u32, RiskMarket>) -> Result<HashMap<Pubkey, Maker>> {
    let idx = owned(s, s.exchange.indices[0], "account:global_trader_index", 96)?;
    let buf = owned(s, s.exchange.buffers[0], "account:active_trader_buffer", 96)?;
    let mut by_node = HashMap::new();
    let mut result = HashMap::new();
    for (node, o) in tree_nodes(idx, 64)? {
        let trader = key(idx, o + 16)?;
        if result
            .insert(
                trader,
                Maker {
                    collateral: i64_at(idx, o + 48)? as i128,
                    margin: 0,
                    base: 0,
                    supported: u32_at(idx, o + 56)? == 63,
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate maker index"));
        }
        by_node.insert(node, trader);
    }
    let mut positions = HashSet::new();
    for (_, o) in tree_nodes(buf, 88)? {
        let node = u32_at(buf, o + 16)?;
        let asset = u32_at(buf, o + 20)?;
        let trader = by_node
            .get(&node)
            .ok_or(invalid("orphan active position"))?;
        let maker = result
            .get_mut(trader)
            .ok_or(invalid("missing active maker"))?;
        if !positions.insert((node, asset)) {
            return Err(invalid("duplicate active position"));
        }
        // Positive spot collateral adds no credit to this lower bound. Locked
        // reserves and unknown extensions need their own rules before inclusion.
        if asset == 0xffff_0000 || asset & 0xffff_0000 == 0xfffe_0000 {
            continue;
        }
        if asset >= 0xff00_0000 {
            maker.supported = false;
            continue;
        }
        let p = o + 24;
        let base = i64_at(buf, p)?;
        let quote = i64_at(buf, p + 8)?;
        if asset as u64 == s.config.asset {
            maker.base = base;
        }
        // Spline accounts cannot carry FIFO orders. Reject any inconsistent state.
        if u32_at(buf, p + 36)? != 0
            || u32_at(buf, p + 44)? != 0
            || u32_at(buf, p + 48)? != 0
            || u32_at(buf, p + 56)? != 0
        {
            maker.supported = false;
        }
        if base == 0 && quote == 0 {
            continue;
        }
        let Some(risk) = risks.get(&asset) else {
            maker.supported = false;
            continue;
        };
        if !risk.fresh {
            maker.supported = false;
        }
        let pnl = base as i128 * risk.mark as i128 + quote as i128;
        let funding = base as i128 * (risk.funding as i128 - i64_at(buf, p + 16)? as i128);
        maker.collateral = maker
            .collateral
            .checked_add(pnl.min(0))
            .and_then(|n| n.checked_sub(funding))
            .ok_or(invalid("maker collateral overflow"))?;
        maker.margin = maker
            .margin
            .checked_add(risk.margin(base.unsigned_abs()))
            .ok_or(invalid("maker margin overflow"))?;
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Fifo {
    price: u64,
    lots: u64,
}
fn fifo(d: &[u8], side: usize) -> Result<Vec<Fifo>> {
    // OrderbookHeader 5504, each side: header 32 + 8192 * 72-byte nodes.
    let start = 5504 + side * 589856;
    if d.len() != 1_185_216 {
        return Err(invalid("unsupported FIFO layout"));
    }
    let count = u64_at(d, start + 16)? as usize;
    if count > 8192 {
        return Err(invalid("invalid FIFO count"));
    }
    let mut ptr = u32_at(d, start)?;
    let mut seen = HashSet::new();
    let mut orders = Vec::new();
    while ptr != 0 {
        if ptr > 8192 || !seen.insert(ptr) || seen.len() > count {
            return Err(invalid("invalid FIFO pointer"));
        }
        let o = start + 32 + (ptr as usize - 1) * 72;
        orders.push(Fifo {
            price: u64_at(d, o + 8)?,
            lots: u64_at(d, o + 40)?,
        });
        ptr = u32_at(d, o + 4)?;
    }
    if orders.len() != count {
        return Err(invalid("FIFO count mismatch"));
    }
    Ok(orders)
}
fn sum_crossed(orders: &[Fifo], mid: u64, bids: bool) -> Result<u64> {
    orders
        .iter()
        .filter(|o| if bids { o.price >= mid } else { o.price <= mid })
        .try_fold(0u64, |n, o| {
            n.checked_add(o.lots).ok_or(invalid("FIFO size overflow"))
        })
}

fn token_balance(s: &State, address: Pubkey, mint: Pubkey, authority: Pubkey) -> Result<u64> {
    let a = get(s, address)?;
    let d = &a.data;
    if a.owner != spl_token::ID
        || a.executable
        || d.len() != 165
        || key(d, 0)? != mint
        || key(d, 32)? != authority
        || d[108] != 1
        || u32_at(d, 72)? != 0
        || u32_at(d, 129)? != 0
    {
        return Err(invalid("invalid Phoenix/Ember token vault"));
    }
    u64_at(d, 64)
}

#[derive(Default)]
pub struct NativePhoenixFactory;
impl PhoenixQuoteFactory for NativePhoenixFactory {
    fn prepare(&self, s: &State) -> Result<Arc<dyn PhoenixQuoter>> {
        super::midpoint::prepare(s)
    }
}

pub struct NativePhoenix {
    curves: [Option<Curve>; 2],
    depth: SplineDepth,
    position: Position,
    risk: RiskMarket,
    tick: u64,
    fee_micro: u64,
    withdrawal_capacity: u64,
    best_bid: u64,
    best_ask: u64,
    can_mint: bool,
    can_redeem: bool,
}
impl NativePhoenix {
    pub fn prepare(s: &State) -> Result<Self> {
        let g = owned(s, PHOENIX_GLOBAL, "account:global_configuration", 1104)?;
        let restart = u64_at(
            &get(s, solana_sdk_ids::sysvar::last_restart_slot::ID)?.data,
            0,
        )?;
        if g[504] & 7 != 3
            || g[505] != 6
            || (u64_at(g, 1096)? != 0 && u64_at(g, 1096)? != restart)
            || u16_at(g, 506)? != 0
        {
            return Err(invalid("unsupported or inactive Phoenix global state"));
        }
        let book = owned(s, s.config.book, "account:orderbook", 5504)?;
        if book[8] != 1
            || book[9] as i8 != s.config.decimals
            || u32_at(book, 32)? as u64 != s.config.asset
            || u64_at(book, 56)? != s.config.tick_size
        {
            return Err(invalid("inactive or mismatched Phoenix book"));
        }
        let child = owned(s, s.config.trader, "account:trader", 240)?;
        let parent = owned(s, s.config.parent, "account:trader", 240)?;
        // Ocelot's current canonical path uses cold, single-position isolated
        // children and an empty cold deposit parent. Hot children require using
        // the buffer for NAV as well; do not silently quote their stale account.
        let flags = u32_at(child, 96)?;
        if flags & 1 != 0
            || flags & !63 != 0
            || u16_at(child, 152)? != 0
            || child[200..224].iter().any(|v| *v != 0)
            || u32_at(child, 108)? != 0
            || key(parent, 24)? != s.config.parent
            || key(parent, 56)? != s.config.authority
        {
            return Err(invalid("unsupported Ocelot trader state"));
        }
        let count = u64_at(child, 224)?;
        let capacity = u64_at(child, 232)?;
        if count > capacity
            || capacity > ((child.len() - 240) / 40) as u64
            || count > u32_at(child, 112)? as u64
            || u64_at(child, 192)? > s.clock.slot
        {
            return Err(invalid("invalid child position capacity or clock"));
        }
        let multiplier = child[103] as i8 as i16 + 10;
        if multiplier < 0 {
            return Err(invalid("negative taker fee multiplier"));
        }
        let fee_micro = (u32_at(book, 96)? as u64 * multiplier as u64).div_ceil(10);
        if fee_micro >= 1_000_000 {
            return Err(invalid("unsupported taker fee"));
        }
        let risks = markets(s)?;
        let mut risk = risks
            .get(&(s.config.asset as u32))
            .ok_or(invalid("missing target risk"))?
            .clone();
        if !risk.fresh || risk.mark as u128 != s.mark.quote_per_lot {
            return Err(invalid("stale native risk state"));
        }
        let makers = makers(s, &risks)?;
        if makers.contains_key(&s.config.trader) {
            return Err(invalid("cold child found in hot index"));
        }
        let bids = fifo(book, 0)?;
        let asks = fifo(book, 1)?;
        // Reserve any OI increase from pre-trade FIFO uncrossing. Counting both
        // position sides and every FIFO order intentionally over-reserves it.
        let fifo_lots = bids.iter().chain(&asks).try_fold(0u64, |n, o| {
            n.checked_add(o.lots)
                .ok_or(invalid("FIFO aggregate overflow"))
        })?;
        risk.oi_room = risk.oi_room.saturating_sub(fifo_lots.saturating_mul(2));
        let spline_key =
            Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &PHOENIX).0;
        let spline = get(s, spline_key)?;
        // Check raw geometry including future-anchored regions. The SDK uses
        // saturating expiry, so they can still affect uncross/slide prices.
        let raw = SplineDepth::with_limits(spline, s.config.book, s.clock.slot, true, |_, _| {
            Ok([SideLimit {
                lots: u64::MAX,
                skip: 0,
            }; 2])
        })?;
        if matches!((raw.best_bid(),raw.best_ask()),(Some(b),Some(a)) if b>=a) {
            return Err(invalid(
                "crossed splines require Phoenix slide-price support",
            ));
        }
        // Favorable-price bounds for full-exit withdrawal capacity. Raw FIFO
        // prices are useful only as an optimistic bound here, never as output.
        let best_bid = bids
            .iter()
            .map(|o| o.price)
            .chain(raw.best_bid())
            .max()
            .unwrap_or(0);
        let best_ask = asks
            .iter()
            .map(|o| o.price)
            .chain(raw.best_ask())
            .min()
            .unwrap_or(u64::MAX);
        let depth =
            SplineDepth::with_limits(spline, s.config.book, s.clock.slot, true, |r, mid| {
                let zero = [SideLimit { lots: 0, skip: 0 }; 2];
                let Some(maker) = makers.get(&r.trader) else {
                    return Ok(zero);
                };
                if !maker.supported || r.trader == s.config.trader || maker.collateral <= 0 {
                    return Ok(zero);
                }
                // Keep risk-increasing fills within the first tier. Reserve the
                // entire pre-uncross position margin, without credit for closing it.
                let size_room = risk.tiers[0].0.saturating_sub(maker.base.unsigned_abs());
                let effective = (maker.collateral as u128)
                    .checked_mul((10_000 - r.leverage_decrease_bps) as u128)
                    .ok_or(invalid("maker haircut overflow"))?
                    / 10_000;
                let free = effective.saturating_sub(maker.margin);
                let mid_atoms = mid as u128 * s.config.tick_size as u128;
                let per_lot = (risk.mark as u128).div_ceil(risk.tiers[0].1 as u128)
                    + mid_atoms.abs_diff(risk.mark as u128);
                let capacity = u64::try_from((free / per_lot).min(size_room as u128))
                    .map_err(|_| invalid("maker capacity overflow"))?;
                let skip = [
                    sum_crossed(&asks, mid, false)?,
                    sum_crossed(&bids, mid, true)?,
                ];
                // Each maker independently reserves ALL potentially crossed FIFO
                // size, even if another maker or hidden liquidity would take it.
                let reserve = skip[0].saturating_add(skip[1]);
                let available = capacity.saturating_sub(reserve);
                let caps = if r.flags & 1 != 0 {
                    [
                        narrow((r.max_long_lots as i128 - maker.base as i128).max(0))?,
                        narrow((r.max_short_lots as i128 + maker.base as i128).max(0))?,
                    ]
                } else {
                    [u64::MAX; 2]
                };
                Ok([
                    SideLimit {
                        lots: available.min(caps[0].saturating_sub(reserve)),
                        skip: skip[0],
                    },
                    SideLimit {
                        lots: available.min(caps[1].saturating_sub(reserve)),
                        skip: skip[1],
                    },
                ])
            })?;
        let q = owned(s, s.exchange.withdraw_queue, "account:withdraw_queue", 120)?;
        let remaining = u64_at(q, 32)?;
        let max = u64_at(q, 24)?;
        let last = u64_at(q, 48)?;
        if remaining > max || last > s.clock.slot {
            return Err(invalid("invalid withdrawal budget"));
        }
        let budget = (remaining as u128 + (s.clock.slot - last) as u128 * u64_at(q, 40)? as u128)
            .min(max as u128) as u64;
        let ember_vault = Pubkey::find_program_address(&[PHOENIX.as_ref(), b"vault"], &EMBER).0;
        let ember_state = Pubkey::find_program_address(&[PHOENIX.as_ref(), b"state"], &EMBER).0;
        // Ember has no public account decoder in this SDK. Support the exact
        // static 136-byte configuration replayed by all four fixtures; any
        // authority, mint, program or layout change requires explicit review.
        let ember = get(s, ember_state)?;
        let expected = [
            0x34, 0x22, 0x25, 0x3f, 0xc2, 0xe9, 0x98, 0x23, 0xd0, 0xfb, 0x52, 0x92, 0x91, 0x28,
            0x47, 0xe2, 0x7e, 0xd1, 0x75, 0x21, 0x5d, 0x2b, 0xb7, 0x70, 0x36, 0x4b, 0x79, 0xc3,
            0x21, 0xb1, 0x8b, 0xc9,
        ];
        if ember.owner != EMBER
            || ember.executable
            || ember.data.len() != 136
            || Sha256::digest(&ember.data)[..] != expected
        {
            return Err(invalid("unreviewed Ember configuration"));
        }
        let withdrawal_capacity = if u64_at(q, 56)? != 0 { 0 } else { budget }
            .min(token_balance(
                s,
                s.exchange.vault,
                COLLATERAL,
                s.exchange.vault,
            )?)
            .min(token_balance(s, ember_vault, USDC, ember_state)?);
        token_balance(
            s,
            super::instruction::pda(b"phoenix_usdc_vault", s.config.market),
            USDC,
            s.config.authority,
        )?;
        token_balance(
            s,
            super::instruction::pda(b"phoenix_collateral_vault", s.config.market),
            COLLATERAL,
            s.config.authority,
        )?;
        token_balance(s, s.market.fees, USDC, s.config.market)?;
        let parent_ok = u32_at(parent, 96)? == 62
            && i64_at(parent, 88)? == 0
            && u64_at(parent, 224)? == 0
            && u32_at(parent, 108)? == 0
            && u16_at(parent, 152)? == 0
            && !makers.contains_key(&s.config.parent)
            && u64_at(parent, 192)? <= s.clock.slot;
        let deposited = u64_at(child, 192)?;
        let cooldown_done =
            deposited <= s.clock.slot && s.clock.slot - deposited >= u64_at(g, 512)?;
        // Shared account/risk/geometry failures above still invalidate the whole
        // state. A curve representability failure disables only that direction.
        let curve = |mint| match Curve::from_spline_depth(s, &depth, fee_micro, mint) {
            Ok(curve) => curve,
            Err(error) => {
                log::debug!(
                    "Ocelot curve unavailable: market={} mint={mint}: {error}",
                    s.config.market
                );
                None
            }
        };
        Ok(Self {
            curves: [curve(true), curve(false)],
            depth,
            position: s.position,
            risk,
            tick: s.config.tick_size,
            fee_micro,
            withdrawal_capacity,
            best_bid,
            best_ask,
            can_mint: parent_ok && flags & 60 == 60 && u32_at(child, 112)? > 0,
            can_redeem: flags & 36 == 36 && cooldown_done,
        })
    }
}

impl PhoenixQuoter for NativePhoenix {
    fn curve(&self, mint: bool) -> Option<&Curve> {
        self.curves[usize::from(!mint)].as_ref()
    }

    fn quote(&self, o: &Order) -> Result<Fill> {
        if (o.reduce_only && !self.can_redeem) || (!o.reduce_only && !self.can_mint) {
            return Err(invalid(
                "Phoenix trader capability or cooldown prevents trade",
            ));
        }
        let base = self.position.base as i128;
        let delta = if o.buy {
            o.lots as i128
        } else {
            -(o.lots as i128)
        };
        let after = base + delta;
        let after_i64 = i64::try_from(after).map_err(|_| invalid("position overflow"))?;
        if o.reduce_only {
            if o.deposit != 0
                || (base != 0 && (base > 0) == o.buy)
                || o.lots > self.position.base.unsigned_abs()
            {
                return Err(invalid("invalid reduce-only quote"));
            }
        } else if (base != 0 && (base > 0) != o.buy) || o.lots > self.risk.oi_room / 2 {
            return Err(invalid(
                "unsupported position reversal or open-interest cap",
            ));
        }
        let quote = self
            .depth
            .quote_notional(o.lots, o.buy, o.limit_ticks, self.tick)?;
        // Bound fee rounding even if matching splits into one-lot fills. This
        // reserve is intentional underquoting, not a claimed exact Phoenix fee.
        let fee = (quote as u128 * self.fee_micro as u128).div_ceil(1_000_000)
            + if self.fee_micro == 0 {
                0
            } else {
                o.lots.saturating_sub(1) as u128
            };
        let fee = u64::try_from(fee).map_err(|_| invalid("Phoenix fee overflow"))?;
        let realized = if o.reduce_only && base != 0 {
            (if o.buy {
                -(quote as i128)
            } else {
                quote as i128
            }) - self.position.quote as i128 * delta / base
        } else {
            0
        };
        let collateral = (self.position.collateral as i128)
            .checked_sub(base * (self.risk.funding as i128 - self.position.funding as i128))
            .and_then(|n| n.checked_add(o.deposit as i128))
            .and_then(|n| n.checked_add(realized))
            .and_then(|n| n.checked_sub(fee as i128))
            .ok_or(invalid("Phoenix collateral overflow"))?;
        i64::try_from(collateral).map_err(|_| invalid("Phoenix signed collateral overflow"))?;
        // Closing cost basis is proportional to the signed original position.
        let virtual_quote = if o.reduce_only && base != 0 {
            self.position.quote as i128 + self.position.quote as i128 * delta / base
        } else if o.reduce_only {
            0
        } else {
            self.position.quote as i128
                + if o.buy {
                    -(quote as i128)
                } else {
                    quote as i128
                }
        };
        let pnl = (after * self.risk.mark as i128)
            .checked_add(virtual_quote)
            .ok_or(invalid("PnL overflow"))?;
        let margin = self.risk.margin(after_i64.unsigned_abs());
        // Partial closes may realize entry-price dust per maker fill. Reserve
        // an atom per lot for that bound; full closes eliminate all cost basis.
        let dust = if o.reduce_only && after != 0 {
            o.lots
        } else {
            0
        };
        let effective = collateral
            .checked_add(pnl.min(0))
            .and_then(|n| n.checked_sub(dust as i128))
            .ok_or(invalid("effective collateral overflow"))?;
        if effective < 0 || margin > effective as u128 {
            return Err(invalid("insufficient conservative Phoenix margin"));
        }
        let collateral_after = narrow(collateral)?.saturating_sub(dust);
        let collateral_upper_bound = if o.reduce_only && after == 0 && base != 0 {
            let price = if o.buy { self.best_ask } else { self.best_bid };
            let favorable = (o.lots as u128)
                .checked_mul(price as u128)
                .and_then(|v| v.checked_mul(self.tick as u128))
                .and_then(|v| i128::try_from(v).ok())
                .ok_or(invalid("exit price bound overflow"))?;
            let upper = (self.position.collateral as i128)
                .checked_sub(base * (self.risk.funding as i128 - self.position.funding as i128))
                .and_then(|v| v.checked_add(self.position.quote as i128))
                .and_then(|v| v.checked_add(if o.buy { -favorable } else { favorable }))
                .ok_or(invalid("exit collateral bound overflow"))?;
            narrow(upper)?
        } else {
            collateral_after
        };
        let withdrawable_after = narrow(effective)?
            .saturating_sub(u64::try_from(margin).map_err(|_| invalid("margin overflow"))?)
            .min(collateral_after)
            .min(self.withdrawal_capacity);
        Ok(Fill {
            lots: o.lots,
            quote_atoms: quote,
            fee_atoms: fee,
            collateral_after,
            collateral_upper_bound,
            withdrawal_capacity: self.withdrawal_capacity,
            withdrawable_after,
            funding_after: self.risk.funding,
        })
    }
}

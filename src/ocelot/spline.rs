//! Spline geometry and prepared, capacity-limited depth.
//!
//! This is declared liquidity, NOT a certified executable-output lower bound.
//! `from_account` remains a geometry diagnostic. The native provider uses
//! `with_limits` to apply maker capacity and uncross reserves before merging.
//!
//! Layout reference: Ellipsis-Labs/rise-public, commit
//! 2ce47221ef04a8d5879d39d31e82f9e92021753e, rust/accounts/src/spline_collection.rs.
//! Unlike the older vendored SDK, this layout includes bid/ask base widths.
use super::{
    PHOENIX,
    codec::{Result, account, boolean, bytes, invalid, key, u64_at},
    discriminator,
};
use solana_account::Account;
use solana_pubkey::Pubkey;
use std::collections::BTreeMap;

const HEADER: usize = 112;
const SPLINE: usize = 1336;
const REGION: usize = 48;
const MAX_LEVEL_VISITS: usize = 65_536;

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(d, o)?))
}

#[derive(Debug, Clone)]
struct Level {
    price: u64,
    cumulative_lots: u64,
    cumulative_tick_lots: u128,
}

/// Captured per-maker inputs that still require Phoenix risk-capacity handling.
#[derive(Debug, Clone)]
pub struct MakerRiskInputs {
    pub trader: Pubkey,
    pub flags: u32,
    pub leverage_decrease_bps: u32,
    pub max_long_lots: u32,
    pub max_short_lots: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct SideLimit {
    pub lots: u64,
    pub skip: u64,
}

/// Prepared visible spline geometry. A quote consumes no heap and performs no I/O.
#[derive(Debug, Clone)]
pub struct SplineDepth {
    bids: Vec<Level>,
    asks: Vec<Level>,
    pub maker_risk_inputs: Vec<MakerRiskInputs>,
    /// Future-slot entries are excluded, rather than treating their expiry as GTC.
    pub excluded_future_splines: usize,
}

impl SplineDepth {
    pub fn from_account(value: &Account, book: Pubkey, slot: u64) -> Result<Self> {
        Self::with_limits(value, book, slot, false, |_, _| {
            Ok([SideLimit {
                lots: u64::MAX,
                skip: 0,
            }; 2])
        })
    }

    pub(crate) fn with_limits(
        value: &Account,
        book: Pubkey,
        slot: u64,
        include_future: bool,
        mut limits: impl FnMut(&MakerRiskInputs, u64) -> Result<[SideLimit; 2]>,
    ) -> Result<Self> {
        let d = account(
            value,
            PHOENIX,
            &discriminator("account:spline_collection"),
            HEADER,
        )?;
        if key(d, 8)? != book {
            return Err(invalid("spline market binding mismatch"));
        }
        let n = u32_at(d, 72)? as usize;
        let active = u32_at(d, 76)? as usize;
        if active > n || n > (d.len() - HEADER) / SPLINE {
            return Err(invalid("invalid spline count"));
        }
        let mut bids = BTreeMap::<u64, u64>::new();
        let mut asks = BTreeMap::<u64, u64>::new();
        let mut maker_risk_inputs = Vec::new();
        let mut excluded_future_splines = 0;
        let mut visits = 0usize;
        for i in 0..n {
            let o = HEADER + i * SPLINE;
            let mid = u64_at(d, o + 40)?;
            if !boolean(d, o + 32)? || mid == 0 {
                continue;
            }
            let updated = u64_at(d, o + 64)?;
            if updated > slot && !include_future {
                excluded_future_splines += 1;
                continue;
            }
            let risk = MakerRiskInputs {
                trader: key(d, o)?,
                flags: u32_at(d, o + 1096)?,
                leverage_decrease_bps: u32_at(d, o + 1100)?,
                max_long_lots: u32_at(d, o + 1104)?,
                max_short_lots: u32_at(d, o + 1108)?,
            };
            if risk.flags & !1 != 0 || risk.leverage_decrease_bps > 10_000 {
                return Err(invalid("unsupported spline risk flags"));
            }
            let capacities = limits(&risk, mid)?;
            for (side, levels) in [(0usize, &mut bids), (1, &mut asks)] {
                let mut capacity = capacities[side].lots;
                let mut skip = capacities[side].skip;
                let start = u64_at(d, o + 72 + side * 8)?;
                let end = u64_at(d, o + 104 + side * 8)?;
                let width = u64_at(d, o + 1112 + side * 8)?;
                if start > end || end > 10 {
                    return Err(invalid("invalid spline region bounds"));
                }
                let mut previous_end = 0;
                for j in start..end {
                    let r = o + 120 + side * 480 + j as usize * REGION;
                    let first = u64_at(d, r)?;
                    let end = u64_at(d, r + 8)?;
                    let density = u32_at(d, r + 16)? as u64;
                    let total = u64_at(d, r + 24)?;
                    let filled = u32_at(d, r + 32)? as u64;
                    let lifespan = u64_at(d, r + 40)?;
                    if end < first || first < previous_end || filled > total {
                        return Err(invalid("invalid spline region"));
                    }
                    previous_end = end;
                    if (end - first).checked_mul(density) != Some(total) {
                        return Err(invalid("spline region size mismatch"));
                    }
                    if density == 0 || filled == total || slot > updated.saturating_add(lifespan) {
                        continue;
                    }
                    let first_unfilled = first
                        .checked_add(filled / density)
                        .ok_or(invalid("spline offset overflow"))?;
                    let count = usize::try_from(end - first_unfilled)
                        .map_err(|_| invalid("spline depth too large"))?;
                    visits = visits
                        .checked_add(count)
                        .ok_or(invalid("spline depth too large"))?;
                    if visits > MAX_LEVEL_VISITS {
                        return Err(invalid("spline depth too large"));
                    }
                    for offset in first_unfilled..end {
                        let distance = width
                            .checked_add(offset)
                            .ok_or(invalid("spline price overflow"))?;
                        let price = if side == 0 {
                            mid.checked_sub(distance)
                        } else {
                            mid.checked_add(distance)
                        }
                        .filter(|p| *p > 0)
                        .ok_or(invalid("invalid spline price"))?;
                        let mut lots = density
                            - if offset == first_unfilled {
                                filled % density
                            } else {
                                0
                            };
                        let skipped = lots.min(skip);
                        skip -= skipped;
                        lots = (lots - skipped).min(capacity);
                        capacity -= lots;
                        if lots == 0 {
                            continue;
                        }
                        let existing = levels.entry(price).or_default();
                        *existing = existing
                            .checked_add(lots)
                            .ok_or(invalid("spline size overflow"))?;
                    }
                }
            }
            maker_risk_inputs.push(risk);
        }
        Ok(Self {
            bids: cumulative(bids.into_iter().rev())?,
            asks: cumulative(asks.into_iter())?,
            maker_risk_inputs,
            excluded_future_splines,
        })
    }

    pub fn best_bid(&self) -> Option<u64> {
        self.bids.first().map(|l| l.price)
    }

    pub fn best_ask(&self) -> Option<u64> {
        self.asks.first().map(|l| l.price)
    }

    /// Visible size only; maker risk may make executable size smaller.
    pub fn declared_lots(&self, buy: bool) -> u64 {
        self.side(buy).last().map_or(0, |l| l.cumulative_lots)
    }

    fn side(&self, buy: bool) -> &[Level] {
        if buy { &self.asks } else { &self.bids }
    }

    pub(crate) fn curve_levels(&self, buy: bool) -> impl Iterator<Item = (u64, u64, u128)> + '_ {
        self.side(buy)
            .iter()
            .map(|l| (l.price, l.cumulative_lots, l.cumulative_tick_lots))
    }

    /// Walk prepared spline geometry, excluding FIFO/hidden take liquidity.
    /// Returns estimated notional before fees, not a production fill guarantee.
    pub fn quote_notional(
        &self,
        lots: u64,
        buy: bool,
        limit_ticks: u64,
        tick_size: u64,
    ) -> Result<u64> {
        if tick_size == 0 {
            return Err(invalid("zero tick size"));
        }
        if lots == 0 {
            return Ok(0);
        }
        if matches!((self.best_bid(), self.best_ask()), (Some(b), Some(a)) if b >= a) {
            return Err(invalid("crossed spline geometry requires slide prices"));
        }
        let levels = self.side(buy);
        let index = levels.partition_point(|l| l.cumulative_lots < lots);
        let level = levels
            .get(index)
            .ok_or(invalid("insufficient declared spline depth"))?;
        if (buy && level.price > limit_ticks) || (!buy && level.price < limit_ticks) {
            return Err(invalid("spline quote exceeds IOC price limit"));
        }
        let (prior_lots, prior_quote) = if index == 0 {
            (0, 0)
        } else {
            (
                levels[index - 1].cumulative_lots,
                levels[index - 1].cumulative_tick_lots,
            )
        };
        let tick_lots = prior_quote
            .checked_add((lots - prior_lots) as u128 * level.price as u128)
            .ok_or(invalid("spline notional overflow"))?;
        let atoms = tick_lots
            .checked_mul(tick_size as u128)
            .ok_or(invalid("spline notional overflow"))?;
        u64::try_from(atoms).map_err(|_| invalid("spline notional overflow"))
    }
}

fn cumulative(levels: impl Iterator<Item = (u64, u64)>) -> Result<Vec<Level>> {
    let mut result = Vec::new();
    let mut lots = 0u64;
    let mut ticks = 0u128;
    for (price, size) in levels {
        lots = lots
            .checked_add(size)
            .ok_or(invalid("spline size overflow"))?;
        ticks = ticks
            .checked_add(price as u128 * size as u128)
            .ok_or(invalid("spline notional overflow"))?;
        result.push(Level {
            price,
            cumulative_lots: lots,
            cumulative_tick_lots: ticks,
        });
    }
    Ok(result)
}

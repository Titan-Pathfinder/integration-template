//! Explicit approximation for raw locked/crossed Phoenix spline books.
//! The five-bp output discount is policy, NOT a proven execution-output bound.
//! See MIDPOINT_VALIDATION.md. Normal books retain the native provider.
use super::{
    PHOENIX, State,
    codec::{Result, boolean, invalid, u64_at},
    curve::Curve,
    phoenix::NativePhoenix,
    quote::{Fill, Order, PhoenixQuoter},
    spline::{SideLimit, SplineDepth},
};
use solana_pubkey::Pubkey;
use std::sync::Arc;

pub const OUTPUT_DISCOUNT_BPS: u64 = 5;

pub(super) fn prepare(s: &State) -> Result<Arc<dyn PhoenixQuoter>> {
    let key = Pubkey::find_program_address(&[b"spline", s.config.book.as_ref()], &PHOENIX).0;
    let account = s
        .accounts
        .get(&key)
        .ok_or(invalid("missing spline account"))?;
    // Same saturating-expiry policy as the native slide-price check. Invalid
    // account data is an error, never a reason to try the approximation.
    let raw = SplineDepth::with_limits(account, s.config.book, s.clock.slot, true, |_, _| {
        Ok([SideLimit {
            lots: u64::MAX,
            skip: 0,
        }; 2])
    })?;
    let (Some(bid), Some(ask)) = (raw.best_bid(), raw.best_ask()) else {
        return Ok(Arc::new(NativePhoenix::prepare(s)?));
    };
    if bid < ask {
        return Ok(Arc::new(NativePhoenix::prepare(s)?));
    }
    let sum = bid as u128 + ask as u128;
    let sell_mid = (sum / 2) as u64;
    let buy_mid = sum.div_ceil(2) as u64;
    let sell = Arc::new(recenter(s, key, sell_mid)?);
    let buy = if buy_mid == sell_mid {
        sell.clone()
    } else {
        Arc::new(recenter(s, key, buy_mid)?)
    };
    log::debug!(
        "Ocelot midpoint approximation: market={} bid={bid} ask={ask} output_discount_bps={OUTPUT_DISCOUNT_BPS}",
        s.config.market
    );
    Ok(Arc::new(MidpointPhoenix {
        buy,
        sell,
        buy_for_direction: [s.market.long, s.position.base < 0],
    }))
}

fn recenter(s: &State, key: Pubkey, mid: u64) -> Result<NativePhoenix> {
    let mut model = s.clone();
    let data = &mut model
        .accounts
        .get_mut(&key)
        .ok_or(invalid("missing spline model"))?
        .data;
    // The raw parser validated the header, count, owner and size above.
    let count = u32::from_le_bytes(data[72..76].try_into().unwrap()) as usize;
    for i in 0..count {
        let offset = 112 + i * 1336;
        if boolean(data, offset + 32)? && u64_at(data, offset + 40)? > 0 {
            data[offset + 40..offset + 48].copy_from_slice(&mid.to_le_bytes());
        }
    }
    // The venue retains the original State. Native validation, maker/taker
    // risk, FIFO reserves, fee and withdrawal checks run on this model, but
    // recentering does not certify their accuracy versus Phoenix slide prices.
    NativePhoenix::prepare(&model)
}

struct MidpointPhoenix {
    buy: Arc<NativePhoenix>,
    sell: Arc<NativePhoenix>,
    buy_for_direction: [bool; 2],
}
impl MidpointPhoenix {
    fn side(&self, buy: bool) -> &NativePhoenix {
        if buy { &self.buy } else { &self.sell }
    }
}
impl PhoenixQuoter for MidpointPhoenix {
    fn quote(&self, order: &Order) -> Result<Fill> {
        self.side(order.buy).quote(order)
    }
    fn curve(&self, mint: bool) -> Option<&Curve> {
        self.side(self.buy_for_direction[usize::from(!mint)])
            .curve(mint)
    }
    fn is_midpoint_approximation(&self) -> bool {
        true
    }
}

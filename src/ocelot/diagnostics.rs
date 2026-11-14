//! Opt-in capture/inspection helpers. The state-only factory cannot quote.
//! Production midpoint preparation lives in `midpoint`, not this module.
use super::{
    PHOENIX, State,
    codec::{Result, boolean, invalid, u64_at},
    quote::{Fill, Order, PhoenixQuoteFactory, PhoenixQuoter},
    spline::{SideLimit, SplineDepth},
};
use solana_account::Account;
use solana_pubkey::Pubkey;
use std::sync::Arc;

const HEADER: usize = 112;
const SPLINE: usize = 1336;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geometry {
    Normal,
    Locked,
    Crossed,
    OneSided,
}

/// Raw geometry, including future-anchored entries because Phoenix's expiry
/// calculation saturates. This is not a prediction of executable liquidity.
pub struct BookReport {
    pub geometry: Geometry,
    pub depth: SplineDepth,
    pub future_active_makers: Vec<(Pubkey, u64)>,
}

pub fn inspect(value: &Account, book: Pubkey, slot: u64) -> Result<BookReport> {
    let depth = SplineDepth::with_limits(value, book, slot, true, |_, _| {
        Ok([SideLimit {
            lots: u64::MAX,
            skip: 0,
        }; 2])
    })?;
    let geometry = match (depth.best_bid(), depth.best_ask()) {
        (Some(b), Some(a)) if b == a => Geometry::Locked,
        (Some(b), Some(a)) if b > a => Geometry::Crossed,
        (Some(_), Some(_)) => Geometry::Normal,
        _ => Geometry::OneSided,
    };
    // with_limits has already validated the account binding, count and length.
    let n = u32::from_le_bytes(value.data[72..76].try_into().unwrap()) as usize;
    let mut future_active_makers = Vec::new();
    for i in 0..n {
        let o = HEADER + i * SPLINE;
        let updated = u64_at(&value.data, o + 64)?;
        if boolean(&value.data, o + 32)? && u64_at(&value.data, o + 40)? > 0 && updated > slot {
            future_active_makers.push((super::codec::key(&value.data, o)?, updated));
        }
    }
    Ok(BookReport {
        geometry,
        depth,
        future_active_makers,
    })
}

pub fn spline_key(book: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"spline", book.as_ref()], &PHOENIX).0
}

/// Decode dependencies even when native quoting rejects the book. Every quote
/// fails; used only to assemble a coherent snapshot and load a reference VM.
pub struct StateOnlyFactory;
struct NoQuotes;
impl PhoenixQuoteFactory for StateOnlyFactory {
    fn prepare(&self, _: &State) -> Result<Arc<dyn PhoenixQuoter>> {
        Ok(Arc::new(NoQuotes))
    }
}
impl PhoenixQuoter for NoQuotes {
    fn quote(&self, _: &Order) -> Result<Fill> {
        Err(invalid("diagnostic state-only provider cannot quote"))
    }
}

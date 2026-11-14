//! Checked readers for the deployed Ocelot v3 ABI. No repr(C) casts or alignment assumptions.
use super::PROGRAM_ID;
use crate::trading_venue::error::TradingVenueError as Error;
use solana_account::Account;
use solana_pubkey::Pubkey;

pub type Result<T> = std::result::Result<T, Error>;
pub fn invalid(message: &'static str) -> Error {
    Error::DeserializationFailed(message.into())
}
pub fn bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N]> {
    data.get(offset..offset.checked_add(N).ok_or(invalid("offset overflow"))?)
        .and_then(|s| s.try_into().ok())
        .ok_or(invalid("truncated account"))
}
pub fn u64_at(data: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(bytes(data, offset)?))
}
pub fn i64_at(data: &[u8], offset: usize) -> Result<i64> {
    Ok(i64::from_le_bytes(bytes(data, offset)?))
}
pub fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(data, offset)?))
}
pub fn key(data: &[u8], offset: usize) -> Result<Pubkey> {
    Ok(Pubkey::new_from_array(bytes(data, offset)?))
}
pub fn boolean(data: &[u8], offset: usize) -> Result<bool> {
    match bytes::<1>(data, offset)?[0] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("invalid boolean")),
    }
}
pub fn account<'a>(
    value: &'a Account,
    owner: Pubkey,
    discriminator: &[u8; 8],
    len: usize,
) -> Result<&'a [u8]> {
    if value.owner != owner
        || value.executable
        || value.data.len() < len
        || value.data.get(..8) != Some(discriminator)
    {
        return Err(invalid("wrong account owner, discriminator or size"));
    }
    Ok(&value.data)
}

#[derive(Clone, Debug)]
pub struct Market {
    pub id: u16,
    pub usdc: Pubkey,
    pub mint: Pubkey,
    pub fees: Pubkey,
    pub leverage: u8,
    pub long: bool,
    pub paused: bool,
    pub deposits_paused: bool,
    pub shares: u64,
    pub mint_fee_bps: u16,
    pub redeem_fee_bps: u16,
    pub streaming_bps: u16,
    pub fee_timestamp: i64,
    pub liability: u64,
    pub remainder: u64,
    pub deposit_cap: u64,
    pub equity_cap: u64,
}
impl Market {
    pub const DISCRIMINATOR: [u8; 8] = [219, 190, 213, 55, 0, 227, 198, 154];
    pub fn decode(address: Pubkey, value: &Account) -> Result<Self> {
        let d = account(value, PROGRAM_ID, &Self::DISCRIMINATOR, 399)?;
        let id = u16_at(d, 232)?;
        let flags = u64_at(d, 317)?;
        if Pubkey::find_program_address(&[b"market", &id.to_le_bytes()], &PROGRAM_ID).0 != address
            || d[235] != 2
            || !(1..=5).contains(&d[236])
            || d[238] != 0
            || flags & !0x7ff != 0
            || flags & 255 != 1
            || flags & 512 == 0
            || key(d, 325)? != Pubkey::default()
            || key(d, 357)? != Pubkey::default()
        {
            return Err(invalid(
                "unsupported market identity, mode or fee/finance version",
            ));
        }
        let mint = key(d, 136)?;
        if Pubkey::find_program_address(&[b"lt2", &id.to_le_bytes()], &PROGRAM_ID).0 != mint {
            return Err(invalid("LT mint does not match market PDA"));
        }
        let m = Self {
            id,
            usdc: key(d, 104)?,
            mint,
            fees: key(d, 200)?,
            leverage: d[236],
            long: boolean(d, 237)?,
            paused: boolean(d, 271)?,
            deposits_paused: flags & 256 != 0,
            shares: u64_at(d, 247)?,
            mint_fee_bps: u16_at(d, 243)?,
            redeem_fee_bps: u16_at(d, 241)?,
            streaming_bps: u16_at(d, 239)?,
            fee_timestamp: i64_at(d, 263)?,
            liability: u64_at(d, 309)?,
            remainder: u64_at(d, 281)?,
            deposit_cap: u64_at(d, 273)?,
            equity_cap: u64_at(d, 389)?,
        };
        if m.usdc != super::USDC
            || m.mint_fee_bps > 100
            || m.redeem_fee_bps > 1000
            || m.streaming_bps > 1000
        {
            return Err(invalid("unsupported mint or fee configuration"));
        }
        Ok(m)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub market: Pubkey,
    pub book: Pubkey,
    pub authority: Pubkey,
    pub parent: Pubkey,
    pub trader: Pubkey,
    pub asset: u64,
    pub tick_size: u64,
    pub decimals: i8,
}
impl Config {
    pub fn decode(market: Pubkey, value: &Account) -> Result<Self> {
        let disc = super::discriminator("account:PhoenixConfig");
        let d = account(value, PROGRAM_ID, &disc, 191)?;
        let c = Self {
            market: key(d, 8)?,
            book: key(d, 40)?,
            authority: key(d, 72)?,
            parent: key(d, 104)?,
            trader: key(d, 136)?,
            asset: u64_at(d, 168)?,
            tick_size: u64_at(d, 176)?,
            decimals: d[189] as i8,
        };
        let shard = u16_at(d, 184)?;
        if c.market != market
            || c.tick_size == 0
            || c.authority
                != Pubkey::find_program_address(
                    &[b"phoenix_authority", &shard.to_le_bytes()],
                    &PROGRAM_ID,
                )
                .0
            || c.parent
                != Pubkey::find_program_address(
                    &[b"trader", c.authority.as_ref(), &[d[186], d[187]]],
                    &super::PHOENIX,
                )
                .0
            || c.trader
                != Pubkey::find_program_address(
                    &[b"trader", c.authority.as_ref(), &[d[186], d[188]]],
                    &super::PHOENIX,
                )
                .0
        {
            return Err(invalid("Phoenix config binding mismatch"));
        }
        Ok(c)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub ticks: u64,
    pub quote_per_lot: u128,
    pub funding: i64,
    pub slot: u64,
}
pub fn mark(value: &Account, c: &Config, slot: u64) -> Result<Mark> {
    let d = account(
        value,
        super::PHOENIX,
        &[0x8e, 0x36, 0x73, 0xdf, 0x7c, 0xef, 0x6c, 0x26],
        48,
    )?;
    let n = u32::from_le_bytes(bytes(d, 32)?) as usize;
    if n > 1024 || d.len() < 48 + n * 1584 {
        return Err(invalid("invalid Phoenix asset map size"));
    }
    for i in 0..n {
        let o = 64 + i * 1584;
        if d[o + 1450] != 0 {
            continue;
        }
        let asset = u16_at(d, o + 944)? as u64 | ((u16_at(d, o + 948)? as u64) << 16);
        if asset != c.asset {
            continue;
        }
        let ticks = u64_at(d, o + 24)?;
        let observed = u64_at(d, o + 16)?;
        if key(d, o + 904)? != c.book
            || u64_at(d, o + 936)? != c.tick_size
            || d[o + 946] as i8 != c.decimals
            || ticks == 0
            || observed == 0
            || observed > slot
            || slot - observed > 150
        {
            return Err(invalid("stale or mismatched Phoenix mark"));
        }
        return Ok(Mark {
            ticks,
            quote_per_lot: ticks as u128 * c.tick_size as u128,
            funding: i64_at(d, o + 1312)?,
            slot: observed,
        });
    }
    Err(invalid("Phoenix asset absent from map"))
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub collateral: i64,
    pub base: i64,
    pub quote: i64,
    pub funding: i64,
}
pub fn position(value: &Account, c: &Config) -> Result<Position> {
    let d = &value.data;
    if value.owner != super::PHOENIX
        || d.len() < 240
        || key(d, 24)? != c.trader
        || key(d, 56)? != c.authority
    {
        return Err(invalid("invalid Phoenix child trader"));
    }
    let n = u64_at(d, 224)?;
    if n > 1 || d.len() < 240 + n as usize * 40 {
        return Err(invalid("child has unexpected positions"));
    }
    let mut p = Position {
        collateral: i64_at(d, 88)?,
        base: 0,
        quote: 0,
        funding: 0,
    };
    if n == 1 {
        if u64_at(d, 240)? != c.asset {
            return Err(invalid("child asset mismatch"));
        }
        p.base = i64_at(d, 248)?;
        p.quote = i64_at(d, 256)?;
        p.funding = i64_at(d, 264)?;
    }
    Ok(p)
}

pub fn gross(p: Position, mark: Mark, cash: u64) -> Result<u64> {
    let funding = (p.base as i128)
        .checked_mul(mark.funding as i128 - p.funding as i128)
        .ok_or(invalid("funding overflow"))?;
    let notional = (p.base as i128)
        .checked_mul(i128::try_from(mark.quote_per_lot).map_err(|_| invalid("price overflow"))?)
        .ok_or(invalid("notional overflow"))?;
    let venue = (p.collateral as i128)
        .checked_sub(funding)
        .and_then(|v| v.checked_add(notional))
        .and_then(|v| v.checked_add(p.quote as i128))
        .ok_or(invalid("equity overflow"))?;
    if venue < 0 {
        return Err(invalid("negative venue equity"));
    }
    u64::try_from(venue + cash as i128).map_err(|_| invalid("gross equity overflow"))
}
pub fn accrued_liability(m: &Market, gross: u64, now: i64) -> Result<u64> {
    const DENOM: u128 = 10_000 * 365 * 24 * 60 * 60;
    if now < m.fee_timestamp || m.remainder as u128 >= DENOM {
        return Err(invalid("invalid fee clock"));
    }
    if m.shares == 0 {
        return if m.liability == 0 {
            Ok(0)
        } else {
            Err(invalid("zero supply liability"))
        };
    }
    let assets = gross.saturating_sub(m.liability);
    let fee = (assets as u128)
        .checked_mul(m.streaming_bps as u128)
        .and_then(|v| v.checked_mul((now as i128 - m.fee_timestamp as i128) as u128))
        .and_then(|v| v.checked_add(m.remainder as u128))
        .ok_or(invalid("fee overflow"))?
        / DENOM;
    m.liability
        .checked_add(fee.min(assets as u128) as u64)
        .ok_or(invalid("liability overflow"))
}

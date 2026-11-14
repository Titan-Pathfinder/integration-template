//! Concave routing estimates below the native whole-lot issuer calculation.
//!
//! Preparation turns the convex Phoenix execution-cost function into an upper
//! envelope over every possible rounded lot count. Issuer output is the lower
//! envelope of the resulting affine lines. See CURVE_VALIDATION.md for the bound.
//! Only preparation/range discovery uses wide division; quote queries do not.
use super::{
    State,
    codec::{Result, invalid},
    quote::{Limits, Output, limit_ticks},
    spline::SplineDepth,
};

mod wide {
    // The macro supplies arithmetic helpers that trigger these lint patterns.
    #![allow(clippy::manual_div_ceil, clippy::assign_op_pattern)]
    uint::construct_uint! { pub struct U256(4); }
}
use wide::U256;

const MICRO: i128 = 1_000_000;
const BPS: i128 = 10_000;
const SCALE: f64 = 18_446_744_073_709_551_616.0;

fn mul(a: i128, b: i128) -> Result<i128> {
    a.checked_mul(b).ok_or(invalid("curve arithmetic overflow"))
}
fn add(a: i128, b: i128) -> Result<i128> {
    a.checked_add(b).ok_or(invalid("curve arithmetic overflow"))
}
fn sub(a: i128, b: i128) -> Result<i128> {
    a.checked_sub(b).ok_or(invalid("curve arithmetic overflow"))
}

fn wide_add(a: U256, b: U256) -> Result<U256> {
    a.checked_add(b).ok_or(invalid("curve range overflow"))
}
fn product(values: &[U256]) -> Result<U256> {
    values.iter().try_fold(U256::one(), |a, b| {
        a.checked_mul(*b).ok_or(invalid("curve range overflow"))
    })
}

/// Intersect the integer domain with (ap-am)*x + (bp-bm) >= 0, without signed
/// wide arithmetic or float-to-integer rounding at an eligibility boundary.
fn intersect(range: &mut (u64, u64), ap: U256, am: U256, bp: U256, bm: U256) -> Result<()> {
    let empty = || invalid("no continuous mint finance range");
    match ap.cmp(&am) {
        std::cmp::Ordering::Equal => {
            if bp < bm {
                return Err(empty());
            }
        }
        std::cmp::Ordering::Greater if bp < bm => {
            let n = bm - bp;
            let d = ap - am;
            let min = n / d + U256::from(u8::from(!(n % d).is_zero()));
            if min > U256::from(range.1) {
                return Err(empty());
            }
            range.0 = range.0.max(min.as_u64());
        }
        std::cmp::Ordering::Less => {
            if bp < bm {
                return Err(empty());
            }
            let max = (bp - bm) / (am - ap);
            if max < U256::from(range.0) {
                return Err(empty());
            }
            if max < U256::from(range.1) {
                range.1 = max.as_u64();
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Line {
    /// Positive slope, rounded DOWN to unsigned Q64.64 during preparation.
    slope: u128,
    /// Intercept rounded DOWN to whole output atoms. Never floating-point output.
    intercept: i128,
    /// First integer input at which this line is the minimum of the envelope.
    start: u64,
}

impl Line {
    fn new(rate_n: i128, rate_d: i128, offset_n: i128, offset_d: i128) -> Result<Self> {
        if rate_n <= 0 || rate_d <= 0 || offset_d <= 0 {
            return Err(invalid("unsupported nonpositive curve slope"));
        }
        let slope = (U256::from(rate_n as u128) << 64) / U256::from(rate_d as u128);
        if slope.is_zero() || slope > U256::from(u128::MAX) {
            return Err(invalid("curve slope outside fixed-point range"));
        }
        Ok(Self {
            slope: slope.as_u128(),
            intercept: offset_n.div_euclid(offset_d),
            start: 0,
        })
    }

    fn value(&self, amount: u64) -> Result<u64> {
        // Exact floor(slope * amount / 2^64), without a 192-bit hot-path multiply.
        let whole = (self.slope >> 64) * amount as u128;
        let fraction = ((self.slope as u64 as u128) * amount as u128) >> 64;
        let value = whole
            .checked_add(fraction)
            .and_then(|v| i128::try_from(v).ok())
            .and_then(|v| v.checked_add(self.intercept))
            .and_then(|v| u64::try_from(v).ok())
            .filter(|v| *v != 0)
            .ok_or(invalid("nonpositive or overflowing routing output"))?;
        Ok(value)
    }
}

/// Immutable lower envelope, cached with its matching Phoenix state.
#[derive(Clone, Debug)]
pub struct Curve {
    lines: Vec<Line>,
}

impl Curve {
    fn from_lines(mut candidates: Vec<Line>) -> Result<Self> {
        candidates
            .sort_unstable_by(|a, b| b.slope.cmp(&a.slope).then(a.intercept.cmp(&b.intercept)));
        let mut lines: Vec<Line> = Vec::with_capacity(candidates.len());
        for mut next in candidates {
            if lines.last().is_some_and(|p| p.slope == next.slope) {
                continue; // Same slope: sorting retained the smaller intercept.
            }
            loop {
                let Some(prev) = lines.last() else {
                    next.start = 0;
                    lines.push(next);
                    break;
                };
                let rise = sub(next.intercept, prev.intercept)?;
                if rise <= 0 {
                    lines.pop(); // Lower slope AND intercept dominates for x >= 0.
                    continue;
                }
                let numerator = U256::from(rise as u128) << 64;
                let denominator = U256::from(prev.slope - next.slope);
                let crossing = numerator / denominator
                    + U256::from(u8::from(!(numerator % denominator).is_zero()));
                if crossing > U256::from(u64::MAX) {
                    break; // This line cannot become active in the integer domain.
                }
                next.start = crossing.as_u64();
                if next.start <= prev.start {
                    lines.pop();
                    continue;
                }
                lines.push(next);
                break;
            }
        }
        if lines.is_empty() {
            return Err(invalid("empty routing curve"));
        }
        Ok(Self { lines })
    }

    pub fn quote(&self, amount: u64) -> Result<Output> {
        let index = self.lines.partition_point(|l| l.start <= amount) - 1;
        let line = &self.lines[index];
        Ok(Output {
            // Zero is Titan's spot-query sentinel, not an executable trade. The
            // positive-input curve is defined on the native executable domain.
            amount: if amount == 0 { 0 } else { line.value(amount)? },
            price: line.slope as f64 / SCALE,
        })
    }

    pub fn segment_count(&self) -> usize {
        self.lines.len()
    }

    /// Integer transition inputs for diagnostics and boundary verification.
    pub fn breakpoints(&self) -> impl Iterator<Item = u64> + '_ {
        self.lines.iter().map(|l| l.start)
    }

    /// A continuous mint domain satisfying both finance inequalities even at
    /// whole-lot jumps. This is range-discovery work, not the quote hot path.
    /// The minimum of the output lines, converted back to equity, bounds added
    /// venue equity. Intersect both constraints for every line of that envelope.
    pub(crate) fn mint_finance_range(&self, s: &State, limits: Limits) -> Result<(u64, u64)> {
        let (p, q) = if s.market.shares == 0 {
            (1, 1)
        } else {
            (s.gross.saturating_sub(s.liability), s.market.shares)
        };
        let p = U256::from(p);
        let q = U256::from(q);
        let scale = U256::one() << 64;
        let k = U256::from(limits.max_venue_leverage_bps);
        let net = BPS
            .checked_sub(s.market.mint_fee_bps as i128)
            .filter(|&v| v > 0)
            .ok_or(invalid("invalid mint range fee"))?;
        let leverage = U256::from(s.market.leverage);
        // n <= (a*x + epsilon)*L/M; epsilon=0 when net deposit is exactly x.
        let exposure = wide_add(
            product(&[
                U256::from(s.position.base.unsigned_abs()),
                U256::from(s.mark.quote_per_lot),
            ])?,
            if s.market.mint_fee_bps == 0 {
                U256::zero()
            } else {
                leverage
            },
        )?;
        let required_slope = product(&[U256::from(net as u128), leverage, q, scale])?;
        let required_offset = product(&[U256::from(BPS as u128), exposure, q])?;
        let mut range = (1, u64::MAX);
        for line in &self.lines {
            let slope = product(&[p, U256::from(line.slope)])?;
            let positive = wide_add(
                product(&[U256::from(s.gross), q])?,
                product(&[p, U256::from(line.intercept.max(0) as u128)])?,
            )?;
            let negative = wide_add(
                product(&[U256::from(limits.cash), q])?,
                product(&[p, U256::from(line.intercept.min(0).unsigned_abs())])?,
            )?;
            // Venue equity >= max(minimum, 1); mints always leave exposure.
            intersect(
                &mut range,
                slope,
                U256::zero(),
                product(&[positive, scale])?,
                product(&[
                    wide_add(
                        negative,
                        product(&[U256::from(limits.min_venue_equity.max(1)), q])?,
                    )?,
                    scale,
                ])?,
            )?;
            // K * venue equity >= 10000 * post-mint marked exposure.
            intersect(
                &mut range,
                product(&[k, slope])?,
                required_slope,
                product(&[k, positive, scale])?,
                product(&[wide_add(product(&[k, negative])?, required_offset)?, scale])?,
            )?;
        }
        Ok(range)
    }

    /// Build from the same capacity-limited depth, tick size and effective fee
    /// used by the native matcher. This bounds the issuer estimate only; callers
    /// must retain whole-lot execution/risk checks for the requested amount.
    pub fn from_spline_depth(
        s: &State,
        depth: &SplineDepth,
        fee_micro: u64,
        mint: bool,
    ) -> Result<Option<Self>> {
        if fee_micro >= MICRO as u64
            || s.mark.quote_per_lot == 0
            || s.market.mint_fee_bps >= BPS as u16
            || s.market.redeem_fee_bps >= BPS as u16
        {
            return Err(invalid("invalid routing curve fee or mark"));
        }
        let shares = s.market.shares as i128;
        let equity = s.gross.saturating_sub(s.liability) as i128;
        if (shares > 0 && equity == 0) || (!mint && shares == 0) {
            return Ok(None);
        }
        let fee = if mint {
            s.market.mint_fee_bps
        } else {
            s.market.redeem_fee_bps
        };
        let net = BPS - fee as i128;
        let base = s.position.base.unsigned_abs() as i128;
        let mut lines = Vec::new();
        if !mint {
            // Partial redemptions never credit positive fill-vs-mark gains.
            // floor(gross*x/S)-floor(liability*x/S) >= equity*x/S - 1.
            lines.push(Line::new(mul(net, equity)?, mul(BPS, shares)?, -net, BPS)?);
            if base == 0 {
                return Ok(Some(Self::from_lines(lines)?));
            }
        }
        let buy = if mint {
            s.market.long
        } else {
            s.position.base < 0
        };
        let limit = limit_ticks(s, buy)?;
        let mark =
            i128::try_from(s.mark.quote_per_lot).map_err(|_| invalid("curve mark overflow"))?;
        let levy = if fee_micro == 0 { 0 } else { MICRO };
        let factor = if buy {
            MICRO + fee_micro as i128
        } else {
            MICRO - fee_micro as i128
        };
        let mut previous_lots = 0i128;
        let mut previous_quote = 0i128;
        for (price, lots, tick_lots) in depth.curve_levels(buy) {
            if (buy && price > limit) || (!buy && price < limit) {
                break;
            }
            let price_atoms = mul(price as i128, s.config.tick_size as i128)?;
            let quote_intercept = sub(previous_quote, mul(price_atoms, previous_lots)?)?;
            let (c, b) = if buy {
                (
                    add(sub(mul(price_atoms, factor)?, mul(mark, MICRO)?)?, levy)?,
                    mul(quote_intercept, factor)?,
                )
            } else {
                (
                    add(sub(mul(mark, MICRO)?, mul(price_atoms, factor)?)?, levy)?,
                    mul(-quote_intercept, factor)?,
                )
            };
            // (c*n+b)/MICRO is a supporting line of convex cost C(n).
            // C bounds rounded fees: ceil(Q*f)+n-1 <= Q*f+n for n >= 1.
            let line = if mint {
                let leverage = s.market.leverage as i128;
                let (numerator, denominator) = if shares == 0 {
                    (1, 1)
                } else {
                    (shares, equity)
                };
                // d=ceil(net*x/BPS); n=floor(d*L/mark).
                // n lies in [z-1,z+L/mark], z=net*x*L/(BPS*mark).
                // With zero issuer fee, d=x exactly and the upper shift is zero.
                let shift = if c < 0 {
                    mul(-c, mark)?
                } else if fee != 0 {
                    mul(c, leverage)?
                } else {
                    0
                };
                Line::new(
                    mul(
                        mul(numerator, net)?,
                        sub(mul(mark, MICRO)?, mul(c, leverage)?)?,
                    )?,
                    mul(mul(mul(denominator, BPS)?, mark)?, MICRO)?,
                    mul(-numerator, add(mul(b, mark)?, shift)?)?,
                    mul(mul(denominator, mark)?, MICRO)?,
                )?
            } else {
                // Only abandoned supply can make the final circulating exit use
                // ceil(base*x/S), rather than floor. Bound that extra lot too.
                let ceil_exit = s.actual_supply < s.market.shares
                    && !(base as u128 * s.actual_supply as u128).is_multiple_of(shares as u128);
                let shift = if c < 0 {
                    -c
                } else if ceil_exit {
                    c
                } else {
                    0
                };
                Line::new(
                    mul(net, sub(mul(equity, MICRO)?, mul(c, base)?)?)?,
                    mul(mul(BPS, shares)?, MICRO)?,
                    mul(-net, add(add(b, shift)?, MICRO)?)?,
                    mul(BPS, MICRO)?,
                )?
            };
            lines.push(line);
            previous_lots = lots as i128;
            previous_quote = mul(
                i128::try_from(tick_lots).map_err(|_| invalid("curve depth overflow"))?,
                s.config.tick_size as i128,
            )?;
        }
        // Missing depth remains a per-direction rejection, not a failed refresh.
        if previous_lots == 0 {
            return Ok(None);
        }
        Ok(Some(Self::from_lines(lines)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finance_halfplanes_use_exact_integer_endpoints() {
        // Compare positive/negative/zero slopes and offsets against direct
        // integer evaluation, including exact and fractional crossings.
        for ap in 0..8i128 {
            for am in 0..8i128 {
                for b in -50..=50i128 {
                    let expected: Vec<u64> = (1..=100)
                        .filter(|&x| (ap - am) * x as i128 + b >= 0)
                        .collect();
                    let mut range = (1, 100);
                    let result = intersect(
                        &mut range,
                        U256::from(ap as u128),
                        U256::from(am as u128),
                        U256::from(b.max(0) as u128),
                        U256::from(b.min(0).unsigned_abs()),
                    );
                    if expected.is_empty() {
                        assert!(result.is_err());
                    } else {
                        result.unwrap();
                        assert_eq!(range, (expected[0], *expected.last().unwrap()));
                    }
                }
            }
        }
        let scale = U256::one() << 192;
        let mut range = (1, u64::MAX);
        intersect(&mut range, scale * 3, scale, U256::zero(), scale * 7).unwrap();
        assert_eq!(range.0, 4);
        intersect(&mut range, scale, scale * 3, scale * 17, U256::zero()).unwrap();
        assert_eq!(range, (4, 8));
        assert!(product(&[U256::MAX, U256::from(2)]).is_err());
    }

    #[test]
    fn fixed_point_lines_round_down_even_at_large_inputs() {
        for (n, d, b, bd) in [(17, 13, -7, 5), (1, 3, 8, 7), (9, 10, -17, 19)] {
            let line = Line::new(n, d, b, bd).unwrap();
            for x in [100, 10_001, 1 << 40, (1 << 53) + 1, u64::MAX / 4] {
                let exact = (n * x as i128 * bd + b * d).div_euclid(d * bd) as u64;
                let output = line.value(x).unwrap();
                assert!(output <= exact);
                assert!(exact - output <= 2, "fixed-point rounding bound");
            }
        }
    }

    #[test]
    fn envelope_matches_brute_force_minimum_and_chords() {
        let mut seed = 19u64;
        for _ in 0..100 {
            let mut candidates = vec![Line::new(3, 1, -10, 1).unwrap()];
            for _ in 0..32 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                candidates.push(
                    Line::new(1 + (seed % 300) as i128, 100, (seed % 10_000) as i128, 3).unwrap(),
                );
            }
            let curve = Curve::from_lines(candidates.clone()).unwrap();
            let mut inputs: Vec<u64> = (100..500).collect();
            inputs.extend([1 << 32, (1 << 53) + 7, u64::MAX / 8]);
            for line in &curve.lines {
                inputs.extend([
                    line.start.saturating_sub(1),
                    line.start,
                    line.start.saturating_add(1),
                ]);
            }
            inputs.sort_unstable();
            inputs.dedup();
            let mut previous: Option<(u64, Output)> = None;
            for x in inputs.into_iter().filter(|&x| x >= 100) {
                let expected = candidates
                    .iter()
                    .map(|l| {
                        let scaled = (U256::from(l.slope) * U256::from(x)) >> 64;
                        scaled.as_u128() as i128 + l.intercept
                    })
                    .min()
                    .unwrap();
                let q = curve.quote(x).unwrap();
                assert_eq!(q.amount as i128, expected);
                if let Some((px, pq)) = previous {
                    assert!(q.amount >= pq.amount && q.price <= pq.price);
                    let chord = (q.amount - pq.amount) as f64 / (x - px) as f64;
                    let tol = 2.0 / (x - px) as f64;
                    assert!(chord <= pq.price * (1.0 + 1e-5) + tol);
                    assert!(chord >= q.price * (1.0 - 1e-5) - tol);
                }
                previous = Some((x, q));
            }
        }
        assert!(Line::new(0, 1, 0, 1).is_err());
        assert!(Line::new(1, 0, 0, 1).is_err());
    }
}

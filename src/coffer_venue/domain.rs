//! A conservative routing domain for the contract's segmented surge fee.
//!
//! The segmented charge can make net output decrease as input increases.
//! Titan requires a monotone output curve and a positive, non-increasing
//! marginal price, so advertise only the prefix on which surge is provably
//! zero. The exact quote remains responsible for all other contract limits.

use crate::coffer::{
    constants::PERCENT_SCALE, errors::ErrorCode, math::max_selloff::check_and_advance,
    state::CofferPool,
};

/// Gross input remaining before the contract enters its surge zone.
///
/// `None` means surge imposes no additional routing limit. `Some(0)` means
/// the current window has no remaining surge-free prefix. This does not
/// search for later zero-fee intervals or include constant nonzero fees.
///
/// The contract skips its entire segmented charge when
/// `floor(effective_after * SCALE / cap) <= threshold`. Thus the last allowed
/// effective amount is `floor((cap * (threshold + 1) - 1) / SCALE)`, which
/// includes the final integer fill cell rather than rounding the threshold
/// down to an input amount. Use the contract's window transition first so
/// decay, rollover, snapshot changes and carryover rescaling all agree.
pub(super) fn surge_free_headroom(
    pool: &CofferPool,
    in_idx: u8,
    now: i64,
) -> Result<Option<u64>, ErrorCode> {
    let idx = in_idx as usize;
    if idx >= pool.token_count as usize || idx >= pool.tokens.len() {
        return Err(ErrorCode::InvalidTokenIndex);
    }
    let slot = &pool.tokens[idx];
    let cfg = &slot.config;
    if cfg.max_selloff_pct == 0
        || cfg.variable_fee_slope_high_pct == 0
        || u64::from(cfg.variable_fee_threshold_pct) >= PERCENT_SCALE
    {
        return Ok(None);
    }

    let mut dynamics = slot.dynamics;
    let virtual_balance = dynamics.virtual_balance;
    let window = match check_and_advance(
        &mut dynamics,
        u64::from(cfg.max_selloff_pct),
        cfg.max_selloff_period_length,
        0,
        virtual_balance,
        now,
    ) {
        Ok(Some(window)) => window,
        Ok(None) => return Ok(None),
        Err(ErrorCode::MaxSelloffExceeded) => return Ok(Some(0)),
        Err(error) => return Err(error),
    };
    if window.max_selloff_cap == 0 {
        return Ok(Some(0));
    }

    // A u64 cap times at most SCALE fits u128. Because threshold < SCALE,
    // the result is below cap and safely fits u64.
    let last_effective = ((u128::from(window.max_selloff_cap)
        * (u128::from(cfg.variable_fee_threshold_pct) + 1)
        - 1)
        / u128::from(PERCENT_SCALE)) as u64;
    Ok(Some(
        last_effective.saturating_sub(window.effective_selloff_before),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coffer::math::surge_fee::calc_surge_fee_pct;

    const START: i64 = 1_000;

    fn pool(cap: u64, threshold: u16) -> CofferPool {
        let mut pool = CofferPool {
            token_count: 2,
            ..Default::default()
        };
        let slot = &mut pool.tokens[0];
        slot.config.max_selloff_pct = 10_000;
        slot.config.max_selloff_period_length = 60;
        slot.config.variable_fee_threshold_pct = threshold;
        slot.config.variable_fee_slope_low_pct = 100;
        slot.config.variable_fee_slope_mid_pct = 500;
        slot.config.variable_fee_slope_high_pct = 3_000;
        slot.dynamics.virtual_balance = cap;
        slot.dynamics.selloff_vb_snapshot = cap;
        slot.dynamics.window_start_timestamp = START;
        pool
    }

    #[test]
    fn boundary_matches_contract_integer_fill_at_small_and_extreme_caps() {
        for cap in [0, 1, 99, 9_999, 10_000, 10_001, u64::MAX] {
            for threshold in [0, 1, 1_234, 8_000, 9_999] {
                let pool = pool(cap, threshold);
                let limit = surge_free_headroom(&pool, 0, START).unwrap().unwrap();
                assert!(limit <= cap);
                assert_eq!(
                    calc_surge_fee_pct(0, limit, cap, threshold, 100, 500, 3_000, 0).unwrap(),
                    0,
                    "cap={cap}, threshold={threshold}, limit={limit}"
                );
                if cap > 0 {
                    let fill = u128::from(limit) * u128::from(PERCENT_SCALE) / u128::from(cap);
                    assert!(fill <= u128::from(threshold));
                    // This is the last zero-fee fill cell for the nonzero
                    // low rate in this fixture, including sub-SCALE caps.
                    let next = limit + 1;
                    assert!(next <= cap);
                    assert!(
                        calc_surge_fee_pct(0, next, cap, threshold, 100, 500, 3_000, 0).unwrap()
                            > 0
                    );
                }
            }
        }
    }

    #[test]
    fn fractional_threshold_keeps_the_last_zero_fee_input_atom() {
        let mut pool = pool(10_003, 1_234);
        pool.tokens[0].dynamics.current_selloff = 1_000;
        // floor(cap * threshold / SCALE) is 1_234, but effective 1_235
        // still rounds to fill 1_234 in the contract.
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(Some(235)));
    }

    #[test]
    fn threshold_zero_keeps_only_the_first_integer_fill_cell() {
        let pool = pool(1_000_000_000_000, 0);
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(Some(99_999_999)));
    }

    #[test]
    fn spent_or_tightened_window_has_no_surge_free_headroom() {
        let mut pool = pool(10_000, 8_000);
        pool.tokens[0].dynamics.current_selloff = 8_000;
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(Some(0)));
        pool.tokens[0].dynamics.current_selloff = 8_001;
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(Some(0)));
        pool.tokens[0].config.max_selloff_pct = 5_000;
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(Some(0)));
    }

    #[test]
    fn live_virtual_balance_and_clock_follow_contract_snapshot_rules() {
        let mut pool = pool(10_000, 8_000);
        let dynamics = &mut pool.tokens[0].dynamics;
        dynamics.virtual_balance = 20_000;
        dynamics.previous_selloff = 1_000;
        dynamics.current_selloff = 2_000;

        // An intra-window balance move does not change the snapshot/cap.
        assert_eq!(surge_free_headroom(&pool, 0, START - 10), Ok(Some(5_000)));
        assert_eq!(surge_free_headroom(&pool, 0, START + 30), Ok(Some(5_500)));
        // At rollover cap doubles, and old current carryover also doubles.
        assert_eq!(surge_free_headroom(&pool, 0, START + 60), Ok(Some(12_001)));
        assert_eq!(surge_free_headroom(&pool, 0, START + 90), Ok(Some(14_001)));
        // After two periods both buckets expire against the current balance.
        assert_eq!(surge_free_headroom(&pool, 0, START + 120), Ok(Some(16_001)));
        // Probing never commits rollover into the cached pool.
        assert_eq!(pool.tokens[0].dynamics.selloff_vb_snapshot, 10_000);
        assert_eq!(pool.tokens[0].dynamics.previous_selloff, 1_000);
        assert_eq!(pool.tokens[0].dynamics.current_selloff, 2_000);
    }

    #[test]
    fn disabled_surge_imposes_no_additional_limit() {
        let mut pool = pool(10_000, 8_000);
        pool.tokens[0].config.variable_fee_slope_high_pct = 0;
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(None));
        pool.tokens[0].config.variable_fee_slope_high_pct = 3_000;
        pool.tokens[0].config.variable_fee_threshold_pct = 10_000;
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(None));
        pool.tokens[0].config.variable_fee_threshold_pct = 8_000;
        pool.tokens[0].config.max_selloff_pct = 0;
        assert_eq!(surge_free_headroom(&pool, 0, START), Ok(None));
    }

    #[test]
    fn invalid_window_configuration_is_an_error() {
        let mut pool = pool(10_000, 8_000);
        pool.tokens[0].config.max_selloff_period_length = 0;
        assert_eq!(
            surge_free_headroom(&pool, 0, START),
            Err(ErrorCode::MaxSelloffInvalidConfig)
        );
        pool.tokens[0].config.max_selloff_period_length = 60;
        pool.tokens[0].config.max_selloff_pct = 10_001;
        assert_eq!(
            surge_free_headroom(&pool, 0, START),
            Err(ErrorCode::MaxSelloffInvalidConfig)
        );
        assert_eq!(
            surge_free_headroom(&pool, 2, START),
            Err(ErrorCode::InvalidTokenIndex)
        );
    }
}

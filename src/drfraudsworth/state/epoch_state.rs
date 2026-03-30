// Raw byte parser for EpochState account data.
//
// EpochState byte layout:
//   [0..8]     Anchor discriminator (sha256("account:EpochState")[0..8])
//   [33..35]   crime_buy_tax_bps (u16)
//   [35..37]   crime_sell_tax_bps (u16)
//   [37..39]   fraud_buy_tax_bps (u16)
//   [39..41]   fraud_sell_tax_bps (u16)

use anyhow::{anyhow, Result};
use sha2::{Sha256, Digest};

use crate::drfraudsworth::constants::EPOCH_STATE_DISCRIMINATOR;

const MIN_LEN: usize = 172;

#[derive(Debug, Clone, Copy)]
pub struct ParsedEpochState {
    pub crime_buy_tax_bps: u16,
    pub crime_sell_tax_bps: u16,
    pub fraud_buy_tax_bps: u16,
    pub fraud_sell_tax_bps: u16,
}

impl ParsedEpochState {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < MIN_LEN {
            return Err(anyhow!(
                "EpochState data too short: {} bytes (need {})",
                data.len(),
                MIN_LEN
            ));
        }

        let disc = &data[0..8];
        if disc != EPOCH_STATE_DISCRIMINATOR {
            return Err(anyhow!(
                "EpochState discriminator mismatch: expected {:?}, got {:?}",
                EPOCH_STATE_DISCRIMINATOR,
                disc
            ));
        }

        Ok(Self {
            crime_buy_tax_bps: u16::from_le_bytes([data[33], data[34]]),
            crime_sell_tax_bps: u16::from_le_bytes([data[35], data[36]]),
            fraud_buy_tax_bps: u16::from_le_bytes([data[37], data[38]]),
            fraud_sell_tax_bps: u16::from_le_bytes([data[39], data[40]]),
        })
    }

    pub fn get_tax_bps(&self, is_crime: bool, is_buy: bool) -> u16 {
        match (is_crime, is_buy) {
            (true, true) => self.crime_buy_tax_bps,
            (true, false) => self.crime_sell_tax_bps,
            (false, true) => self.fraud_buy_tax_bps,
            (false, false) => self.fraud_sell_tax_bps,
        }
    }
}

pub fn compute_epoch_state_discriminator() -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(b"account:EpochState");
    let hash = hasher.finalize();
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_epoch_state(crime_buy: u16, crime_sell: u16, fraud_buy: u16, fraud_sell: u16) -> Vec<u8> {
        let mut data = vec![0u8; MIN_LEN];
        data[0..8].copy_from_slice(&EPOCH_STATE_DISCRIMINATOR);
        data[33..35].copy_from_slice(&crime_buy.to_le_bytes());
        data[35..37].copy_from_slice(&crime_sell.to_le_bytes());
        data[37..39].copy_from_slice(&fraud_buy.to_le_bytes());
        data[39..41].copy_from_slice(&fraud_sell.to_le_bytes());
        data
    }

    #[test]
    fn discriminator_matches_computed() {
        assert_eq!(compute_epoch_state_discriminator(), EPOCH_STATE_DISCRIMINATOR);
    }

    #[test]
    fn parse_known_tax_rates() {
        let data = mock_epoch_state(400, 1400, 1400, 400);
        let parsed = ParsedEpochState::from_bytes(&data).unwrap();
        assert_eq!(parsed.crime_buy_tax_bps, 400);
        assert_eq!(parsed.crime_sell_tax_bps, 1400);
        assert_eq!(parsed.fraud_buy_tax_bps, 1400);
        assert_eq!(parsed.fraud_sell_tax_bps, 400);
    }

    #[test]
    fn get_tax_bps_all_directions() {
        let data = mock_epoch_state(300, 1200, 1500, 500);
        let parsed = ParsedEpochState::from_bytes(&data).unwrap();
        assert_eq!(parsed.get_tax_bps(true, true), 300);
        assert_eq!(parsed.get_tax_bps(true, false), 1200);
        assert_eq!(parsed.get_tax_bps(false, true), 1500);
        assert_eq!(parsed.get_tax_bps(false, false), 500);
    }

    #[test]
    fn reject_bad_discriminator() {
        let mut data = vec![0u8; MIN_LEN];
        data[0..8].copy_from_slice(&[0xFF; 8]);
        assert!(ParsedEpochState::from_bytes(&data).is_err());
    }
}

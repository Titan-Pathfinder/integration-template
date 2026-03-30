// Anchor instruction data serialization for Tax Program and Conversion Vault.
//
// Titan's generate_swap_instruction() needs full Instruction objects,
// which require program_id + accounts + data. This module builds the data bytes.
//
// Anchor discriminators are sha256("global:<instruction_name>")[0..8].

use sha2::{Sha256, Digest};

/// Compute an Anchor instruction discriminator.
///
/// Formula: sha256("global:<name>")[0..8]
fn anchor_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(format!("global:{}", name).as_bytes());
    let hash = hasher.finalize();
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

/// Build instruction data for Tax Program `swap_sol_buy`.
///
/// Layout: [8-byte discriminator] [u64 amount_in LE] [u64 minimum_amount_out LE] [bool is_crime]
///
/// The on-chain handler signature:
///   swap_sol_buy(ctx, amount_in: u64, minimum_output: u64, is_crime: bool)
pub fn build_swap_buy_data(amount_in: u64, minimum_amount_out: u64, is_crime: bool) -> Vec<u8> {
    let disc = anchor_discriminator("swap_sol_buy");
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&minimum_amount_out.to_le_bytes());
    data.push(if is_crime { 1 } else { 0 });
    data
}

/// Build instruction data for Tax Program `swap_sol_sell`.
///
/// Layout: [8-byte discriminator] [u64 amount_in LE] [u64 minimum_amount_out LE] [bool is_crime]
///
/// The on-chain handler signature:
///   swap_sol_sell(ctx, amount_in: u64, minimum_output: u64, is_crime: bool)
pub fn build_swap_sell_data(amount_in: u64, minimum_amount_out: u64, is_crime: bool) -> Vec<u8> {
    let disc = anchor_discriminator("swap_sol_sell");
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&minimum_amount_out.to_le_bytes());
    data.push(if is_crime { 1 } else { 0 });
    data
}

/// Build instruction data for Conversion Vault `convert_v2`.
///
/// Layout: [8-byte discriminator] [u64 amount_in LE]
pub fn build_convert_data(amount_in: u64) -> Vec<u8> {
    let disc = anchor_discriminator("convert_v2");
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_buy_data_is_25_bytes() {
        let data = build_swap_buy_data(1_000_000_000, 0, true);
        assert_eq!(data.len(), 25); // 8 disc + 8 amount + 8 min_out + 1 is_crime
    }

    #[test]
    fn swap_sell_data_is_25_bytes() {
        let data = build_swap_sell_data(1_000_000_000, 0, false);
        assert_eq!(data.len(), 25);
    }

    #[test]
    fn convert_data_is_16_bytes() {
        let data = build_convert_data(10_000);
        assert_eq!(data.len(), 16);
    }

    #[test]
    fn buy_and_sell_have_different_discriminators() {
        let buy = build_swap_buy_data(100, 0, true);
        let sell = build_swap_sell_data(100, 0, true);
        assert_ne!(&buy[0..8], &sell[0..8]);
    }

    #[test]
    fn amount_serialized_as_le() {
        let data = build_swap_buy_data(1_000_000_000, 500_000, true);
        let amount_in = u64::from_le_bytes(data[8..16].try_into().unwrap());
        assert_eq!(amount_in, 1_000_000_000);
        let min_out = u64::from_le_bytes(data[16..24].try_into().unwrap());
        assert_eq!(min_out, 500_000);
        assert_eq!(data[24], 1, "is_crime should be true (1)");
    }

    #[test]
    fn is_crime_false_serialized_as_zero() {
        let data = build_swap_buy_data(100, 0, false);
        assert_eq!(data[24], 0, "is_crime=false should serialize as 0");
    }

    #[test]
    fn discriminator_is_deterministic() {
        let d1 = anchor_discriminator("swap_sol_buy");
        let d2 = anchor_discriminator("swap_sol_buy");
        assert_eq!(d1, d2);
    }

    #[test]
    fn convert_amount_serialized_as_le() {
        let data = build_convert_data(10_000);
        let amount = u64::from_le_bytes(data[8..16].try_into().unwrap());
        assert_eq!(amount, 10_000);
    }
}

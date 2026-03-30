// Protocol constants for the Dr. Fraudsworth Titan adapter.

/// LP fee in basis points (1%).
pub const LP_FEE_BPS: u16 = 100;

/// Conversion rate for the vault (100:1 CRIME/FRAUD:PROFIT).
pub const CONVERSION_RATE: u64 = 100;

/// Token decimals for all Dr. Fraudsworth tokens (CRIME, FRAUD, PROFIT).
pub const TOKEN_DECIMALS: u8 = 6;

/// Decimals for SOL (NATIVE_MINT).
pub const SOL_DECIMALS: u8 = 9;

/// Anchor discriminator for EpochState account.
///
/// Computed as: sha256("account:EpochState")[0..8]
///
/// Known value (hex): bf 3f 8b ed 90 0c df d2
pub const EPOCH_STATE_DISCRIMINATOR: [u8; 8] = [0xbf, 0x3f, 0x8b, 0xed, 0x90, 0x0c, 0xdf, 0xd2];

//! Local mirror of `coffer-pool/src/errors.rs`.
//!
//! The contract declares these through Anchor's `#[error_code]`; this crate
//! cannot depend on `anchor-lang` (its solana-program 3.x line conflicts with
//! the template's 2.x ecosystem), so the enum is restated here with the SAME
//! variants in the SAME order. Anchor numbers variants positionally from 6000,
//! so [`ErrorCode::code`] reproduces the on-chain custom error code exactly —
//! the fixture tests use that to match a LiteSVM revert against the quote's
//! predicted failure. `tests/coffer_source_parity.rs` checks the variant list
//! against the contract source.

/// Anchor's `#[error_code]` base offset.
pub const ANCHOR_ERROR_OFFSET: u32 = 6000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCode {
    /// "Invalid token count. Must be between 2 and 10"
    InvalidTokenCount,
    /// "Invalid token index"
    InvalidTokenIndex,
    /// "Invalid weights. Must sum to 10000 (WEIGHT_SCALE)"
    InvalidWeights,
    /// "Invalid virtual balances. Must be greater than zero"
    InvalidVirtualBalances,
    /// "Insufficient liquidity in the pool"
    InsufficientLiquidity,
    /// "Slippage tolerance exceeded"
    SlippageExceeded,
    /// "Invalid amounts provided"
    InvalidAmounts,
    /// "Insufficient BPT minted"
    InsufficientBptOut,
    /// "Insufficient tokens received"
    InsufficientTokensOut,
    /// "Fee rate exceeds maximum"
    FeeRateMaxExceeded,
    /// "Protocol fee rate exceeds maximum"
    ProtocolFeeRateMaxExceeded,
    /// "Math overflow"
    MathOverflow,
    /// "Math underflow"
    MathUnderflow,
    /// "Division by zero"
    DivisionByZero,
    /// "Token mint mismatch"
    TokenMintMismatch,
    /// "Invalid token decimals"
    InvalidTokenDecimals,
    /// "Amount out exceeds available balance"
    AmountOutExceedsBalance,
    /// "Invalid BPT amount"
    InvalidBptAmount,
    /// "Unauthorized"
    Unauthorized,
    /// "Invalid mint account"
    InvalidMint,
    /// "Pool is disabled. No operations allowed."
    PoolDisabled,
    /// "Swaps are disabled for this pool."
    SwapsDisabled,
    /// "Pool must be disabled before debug operations."
    PoolMustBeDisabled,
    /// "Amount must be greater than zero"
    ZeroAmount,
    /// "Swap amount is too small: fee rounds to zero"
    ZeroFeeAmount,
    /// "Token mint has a banned Token-2022 extension"
    BannedExtension,
    /// "Token program does not match pool's token program"
    TokenProgramMismatch,
    /// "User token account owner mismatch"
    UserTokenAccountOwnerMismatch,
    /// "Initial liquidity too small - pool would be bricked"
    InitialLiquidityTooSmall,
    /// "Invalid token program - must be SPL Token or Token-2022"
    InvalidTokenProgram,
    /// "Invalid vault account"
    InvalidVault,
    /// "Source account must be owned by this program"
    InvalidSourceOwner,
    /// "Withdrawal would break rent exemption"
    WouldBreakRentExempt,
    /// "Pool admin is disabled"
    PoolAdminDisabled,
    /// "Protocol admin is unset"
    ProtocolAdminUnset,
    /// "No pending admin transfer"
    NoPendingAdmin,
    /// "Signer is not the pending admin"
    NotPendingAdmin,
    /// "Pending admin cannot be the zero pubkey"
    InvalidPendingAdmin,
    /// "Deprecated: config admin is now pinned by account constraints, not this check"
    ProtocolAdminNotTreasury,
    /// "Range manager is not enabled for this pool"
    RangeManagerDisabled,
    /// "Signer is not the configured range manager"
    RangeManagerUnauthorized,
    /// "Range manager update too frequent — minimum interval not yet elapsed"
    RangeManagerUpdateTooFrequent,
    /// "Range manager virtual balance change exceeds configured percent cap"
    RangeManagerVbChangeTooLarge,
    /// "Range manager weight change exceeds configured percent cap"
    RangeManagerWeightChangeTooLarge,
    /// "Range manager update must change at least one virtual balance or weight"
    RangeManagerEmptyUpdate,
    /// "Range manager percent parameter exceeds 10000"
    RangeManagerInvalidPct,
    /// "Resulting weights do not sum to WEIGHT_SCALE after range manager update"
    RangeManagerWeightSumInvalid,
    /// "Duplicate token index in range manager update"
    RangeManagerDuplicateIndex,
    /// "max_selloff_period_length must be > 0 when max_selloff > 0"
    MaxSelloffInvalidConfig,
    /// "Token max-selloff threshold exceeded for current window"
    MaxSelloffExceeded,
    /// "Pool already has an Address Lookup Table initialised"
    AltAlreadyInitialized,
    /// "ALT address mismatch — re-derived from recent_slot doesn't match passed account"
    AltAddressMismatch,
    /// "Invalid Address Lookup Table program id"
    InvalidAltProgram,
    /// "Invalid surge fee curve config"
    InvalidSurgeFeeConfig,
    /// "Token is deactivated for swaps on the input side"
    TokenInactive,
    /// "Mint has an unsupported Token-2022 extension"
    UnsupportedExtension,
    /// "Destination requires a memo; pass the SPL Memo program account"
    MemoProgramMissing,
    /// "Transfer fee round-trip verification failed"
    TransferFeeCalculationMismatch,
    /// "Deposit amount is fully consumed by the token transfer fee"
    AmountFullyConsumedByTransferFee,
    /// "Only the pool admin may make the pool's first (seed) deposit"
    SeedDepositNotPoolAdmin,
    /// "Deprecated: see single-token-liquidity's own TokenExtensionsUnsupported"
    StldTokenExtensionsUnsupported,
    /// "Range manager update is stale — expected_current does not match stored value"
    RangeManagerStaleValue,
    /// "Range manager update exceeds the configured leverage band"
    RangeManagerLeverageBandExceeded,
    /// "Protocol-admin range-manager override may only disable, not appoint"
    RangeManagerOverrideMustDisable,
    /// "Array length does not match the pool's token count"
    InvalidArrayLength,
    /// "First deposit must fund at least one token"
    FirstDepositRequiresNonzero,
    /// "Deposit would change a token's live/sidelined state"
    TokenLivenessMismatch,
    /// "Deposit is too small to mint any BPT"
    DepositTooSmall,
    /// "Pool holds no liquidity: every actual_balance is zero"
    PoolNotSeeded,
}

impl ErrorCode {
    /// The on-chain custom error code (`6000 + variant index`).
    pub fn code(self) -> u32 {
        ANCHOR_ERROR_OFFSET + self as u32
    }

    /// Stable variant name, for `'static` error context without allocating.
    pub fn name(self) -> &'static str {
        match self {
            ErrorCode::InvalidTokenCount => "InvalidTokenCount",
            ErrorCode::InvalidTokenIndex => "InvalidTokenIndex",
            ErrorCode::InvalidWeights => "InvalidWeights",
            ErrorCode::InvalidVirtualBalances => "InvalidVirtualBalances",
            ErrorCode::InsufficientLiquidity => "InsufficientLiquidity",
            ErrorCode::SlippageExceeded => "SlippageExceeded",
            ErrorCode::InvalidAmounts => "InvalidAmounts",
            ErrorCode::InsufficientBptOut => "InsufficientBptOut",
            ErrorCode::InsufficientTokensOut => "InsufficientTokensOut",
            ErrorCode::FeeRateMaxExceeded => "FeeRateMaxExceeded",
            ErrorCode::ProtocolFeeRateMaxExceeded => "ProtocolFeeRateMaxExceeded",
            ErrorCode::MathOverflow => "MathOverflow",
            ErrorCode::MathUnderflow => "MathUnderflow",
            ErrorCode::DivisionByZero => "DivisionByZero",
            ErrorCode::TokenMintMismatch => "TokenMintMismatch",
            ErrorCode::InvalidTokenDecimals => "InvalidTokenDecimals",
            ErrorCode::AmountOutExceedsBalance => "AmountOutExceedsBalance",
            ErrorCode::InvalidBptAmount => "InvalidBptAmount",
            ErrorCode::Unauthorized => "Unauthorized",
            ErrorCode::InvalidMint => "InvalidMint",
            ErrorCode::PoolDisabled => "PoolDisabled",
            ErrorCode::SwapsDisabled => "SwapsDisabled",
            ErrorCode::PoolMustBeDisabled => "PoolMustBeDisabled",
            ErrorCode::ZeroAmount => "ZeroAmount",
            ErrorCode::ZeroFeeAmount => "ZeroFeeAmount",
            ErrorCode::BannedExtension => "BannedExtension",
            ErrorCode::TokenProgramMismatch => "TokenProgramMismatch",
            ErrorCode::UserTokenAccountOwnerMismatch => "UserTokenAccountOwnerMismatch",
            ErrorCode::InitialLiquidityTooSmall => "InitialLiquidityTooSmall",
            ErrorCode::InvalidTokenProgram => "InvalidTokenProgram",
            ErrorCode::InvalidVault => "InvalidVault",
            ErrorCode::InvalidSourceOwner => "InvalidSourceOwner",
            ErrorCode::WouldBreakRentExempt => "WouldBreakRentExempt",
            ErrorCode::PoolAdminDisabled => "PoolAdminDisabled",
            ErrorCode::ProtocolAdminUnset => "ProtocolAdminUnset",
            ErrorCode::NoPendingAdmin => "NoPendingAdmin",
            ErrorCode::NotPendingAdmin => "NotPendingAdmin",
            ErrorCode::InvalidPendingAdmin => "InvalidPendingAdmin",
            ErrorCode::ProtocolAdminNotTreasury => "ProtocolAdminNotTreasury",
            ErrorCode::RangeManagerDisabled => "RangeManagerDisabled",
            ErrorCode::RangeManagerUnauthorized => "RangeManagerUnauthorized",
            ErrorCode::RangeManagerUpdateTooFrequent => "RangeManagerUpdateTooFrequent",
            ErrorCode::RangeManagerVbChangeTooLarge => "RangeManagerVbChangeTooLarge",
            ErrorCode::RangeManagerWeightChangeTooLarge => "RangeManagerWeightChangeTooLarge",
            ErrorCode::RangeManagerEmptyUpdate => "RangeManagerEmptyUpdate",
            ErrorCode::RangeManagerInvalidPct => "RangeManagerInvalidPct",
            ErrorCode::RangeManagerWeightSumInvalid => "RangeManagerWeightSumInvalid",
            ErrorCode::RangeManagerDuplicateIndex => "RangeManagerDuplicateIndex",
            ErrorCode::MaxSelloffInvalidConfig => "MaxSelloffInvalidConfig",
            ErrorCode::MaxSelloffExceeded => "MaxSelloffExceeded",
            ErrorCode::AltAlreadyInitialized => "AltAlreadyInitialized",
            ErrorCode::AltAddressMismatch => "AltAddressMismatch",
            ErrorCode::InvalidAltProgram => "InvalidAltProgram",
            ErrorCode::InvalidSurgeFeeConfig => "InvalidSurgeFeeConfig",
            ErrorCode::TokenInactive => "TokenInactive",
            ErrorCode::UnsupportedExtension => "UnsupportedExtension",
            ErrorCode::MemoProgramMissing => "MemoProgramMissing",
            ErrorCode::TransferFeeCalculationMismatch => "TransferFeeCalculationMismatch",
            ErrorCode::AmountFullyConsumedByTransferFee => "AmountFullyConsumedByTransferFee",
            ErrorCode::SeedDepositNotPoolAdmin => "SeedDepositNotPoolAdmin",
            ErrorCode::StldTokenExtensionsUnsupported => "StldTokenExtensionsUnsupported",
            ErrorCode::RangeManagerStaleValue => "RangeManagerStaleValue",
            ErrorCode::RangeManagerLeverageBandExceeded => "RangeManagerLeverageBandExceeded",
            ErrorCode::RangeManagerOverrideMustDisable => "RangeManagerOverrideMustDisable",
            ErrorCode::InvalidArrayLength => "InvalidArrayLength",
            ErrorCode::FirstDepositRequiresNonzero => "FirstDepositRequiresNonzero",
            ErrorCode::TokenLivenessMismatch => "TokenLivenessMismatch",
            ErrorCode::DepositTooSmall => "DepositTooSmall",
            ErrorCode::PoolNotSeeded => "PoolNotSeeded",
        }
    }

    /// The contract's `#[msg(..)]` text.
    pub fn msg(self) -> &'static str {
        match self {
            ErrorCode::InvalidTokenCount => "Invalid token count. Must be between 2 and 10",
            ErrorCode::InvalidTokenIndex => "Invalid token index",
            ErrorCode::InvalidWeights => "Invalid weights. Must sum to 10000 (WEIGHT_SCALE)",
            ErrorCode::InvalidVirtualBalances => {
                "Invalid virtual balances. Must be greater than zero"
            }
            ErrorCode::InsufficientLiquidity => "Insufficient liquidity in the pool",
            ErrorCode::SlippageExceeded => "Slippage tolerance exceeded",
            ErrorCode::InvalidAmounts => "Invalid amounts provided",
            ErrorCode::InsufficientBptOut => "Insufficient BPT minted",
            ErrorCode::InsufficientTokensOut => "Insufficient tokens received",
            ErrorCode::FeeRateMaxExceeded => "Fee rate exceeds maximum",
            ErrorCode::ProtocolFeeRateMaxExceeded => "Protocol fee rate exceeds maximum",
            ErrorCode::MathOverflow => "Math overflow",
            ErrorCode::MathUnderflow => "Math underflow",
            ErrorCode::DivisionByZero => "Division by zero",
            ErrorCode::TokenMintMismatch => "Token mint mismatch",
            ErrorCode::InvalidTokenDecimals => "Invalid token decimals",
            ErrorCode::AmountOutExceedsBalance => "Amount out exceeds available balance",
            ErrorCode::InvalidBptAmount => "Invalid BPT amount",
            ErrorCode::Unauthorized => "Unauthorized",
            ErrorCode::InvalidMint => "Invalid mint account",
            ErrorCode::PoolDisabled => "Pool is disabled. No operations allowed.",
            ErrorCode::SwapsDisabled => "Swaps are disabled for this pool.",
            ErrorCode::PoolMustBeDisabled => "Pool must be disabled before debug operations.",
            ErrorCode::ZeroAmount => "Amount must be greater than zero",
            ErrorCode::ZeroFeeAmount => "Swap amount is too small: fee rounds to zero",
            ErrorCode::BannedExtension => "Token mint has a banned Token-2022 extension",
            ErrorCode::TokenProgramMismatch => "Token program does not match pool's token program",
            ErrorCode::UserTokenAccountOwnerMismatch => "User token account owner mismatch",
            ErrorCode::InitialLiquidityTooSmall => {
                "Initial liquidity too small - pool would be bricked"
            }
            ErrorCode::InvalidTokenProgram => {
                "Invalid token program - must be SPL Token or Token-2022"
            }
            ErrorCode::InvalidVault => "Invalid vault account",
            ErrorCode::InvalidSourceOwner => "Source account must be owned by this program",
            ErrorCode::WouldBreakRentExempt => "Withdrawal would break rent exemption",
            ErrorCode::PoolAdminDisabled => "Pool admin is disabled",
            ErrorCode::ProtocolAdminUnset => "Protocol admin is unset",
            ErrorCode::NoPendingAdmin => "No pending admin transfer",
            ErrorCode::NotPendingAdmin => "Signer is not the pending admin",
            ErrorCode::InvalidPendingAdmin => "Pending admin cannot be the zero pubkey",
            ErrorCode::ProtocolAdminNotTreasury => {
                "Deprecated: config admin is now pinned by account constraints, not this check"
            }
            ErrorCode::RangeManagerDisabled => "Range manager is not enabled for this pool",
            ErrorCode::RangeManagerUnauthorized => "Signer is not the configured range manager",
            ErrorCode::RangeManagerUpdateTooFrequent => {
                "Range manager update too frequent — minimum interval not yet elapsed"
            }
            ErrorCode::RangeManagerVbChangeTooLarge => {
                "Range manager virtual balance change exceeds configured percent cap"
            }
            ErrorCode::RangeManagerWeightChangeTooLarge => {
                "Range manager weight change exceeds configured percent cap"
            }
            ErrorCode::RangeManagerEmptyUpdate => {
                "Range manager update must change at least one virtual balance or weight"
            }
            ErrorCode::RangeManagerInvalidPct => "Range manager percent parameter exceeds 10000",
            ErrorCode::RangeManagerWeightSumInvalid => {
                "Resulting weights do not sum to WEIGHT_SCALE after range manager update"
            }
            ErrorCode::RangeManagerDuplicateIndex => {
                "Duplicate token index in range manager update"
            }
            ErrorCode::MaxSelloffInvalidConfig => {
                "max_selloff_period_length must be > 0 when max_selloff > 0"
            }
            ErrorCode::MaxSelloffExceeded => {
                "Token max-selloff threshold exceeded for current window"
            }
            ErrorCode::AltAlreadyInitialized => {
                "Pool already has an Address Lookup Table initialised"
            }
            ErrorCode::AltAddressMismatch => {
                "ALT address mismatch — re-derived from recent_slot doesn't match passed account"
            }
            ErrorCode::InvalidAltProgram => "Invalid Address Lookup Table program id",
            ErrorCode::InvalidSurgeFeeConfig => "Invalid surge fee curve config",
            ErrorCode::TokenInactive => "Token is deactivated for swaps on the input side",
            ErrorCode::UnsupportedExtension => "Mint has an unsupported Token-2022 extension",
            ErrorCode::MemoProgramMissing => {
                "Destination requires a memo; pass the SPL Memo program account"
            }
            ErrorCode::TransferFeeCalculationMismatch => {
                "Transfer fee round-trip verification failed"
            }
            ErrorCode::AmountFullyConsumedByTransferFee => {
                "Deposit amount is fully consumed by the token transfer fee"
            }
            ErrorCode::SeedDepositNotPoolAdmin => {
                "Only the pool admin may make the pool's first (seed) deposit"
            }
            ErrorCode::StldTokenExtensionsUnsupported => {
                "Deprecated: see single-token-liquidity's own TokenExtensionsUnsupported"
            }
            ErrorCode::RangeManagerStaleValue => {
                "Range manager update is stale — expected_current does not match stored value"
            }
            ErrorCode::RangeManagerLeverageBandExceeded => {
                "Range manager update exceeds the configured leverage band"
            }
            ErrorCode::RangeManagerOverrideMustDisable => {
                "Protocol-admin range-manager override may only disable, not appoint"
            }
            ErrorCode::InvalidArrayLength => "Array length does not match the pool's token count",
            ErrorCode::FirstDepositRequiresNonzero => "First deposit must fund at least one token",
            ErrorCode::TokenLivenessMismatch => {
                "Deposit would change a token's live/sidelined state"
            }
            ErrorCode::DepositTooSmall => "Deposit is too small to mint any BPT",
            ErrorCode::PoolNotSeeded => "Pool holds no liquidity: every actual_balance is zero",
        }
    }
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ({}): {}", self.name(), self.code(), self.msg())
    }
}

impl std::error::Error for ErrorCode {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codes pinned by the contract's own `deployed_error_codes_never_move` test.
    #[test]
    fn codes_match_the_deployed_numbering() {
        assert_eq!(ErrorCode::InvalidTokenCount.code(), 6000);
        assert_eq!(ErrorCode::MathOverflow.code(), 6011);
        assert_eq!(ErrorCode::AmountOutExceedsBalance.code(), 6016);
        assert_eq!(ErrorCode::ZeroFeeAmount.code(), 6024);
        assert_eq!(ErrorCode::MaxSelloffExceeded.code(), 6049);
        assert_eq!(ErrorCode::TokenInactive.code(), 6054);
        assert_eq!(ErrorCode::StldTokenExtensionsUnsupported.code(), 6060);
    }
}

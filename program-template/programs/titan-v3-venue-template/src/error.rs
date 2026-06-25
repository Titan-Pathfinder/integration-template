use anchor_lang::prelude::*;

#[error_code]
pub enum TemplateError {
    #[msg("Invalid swap input")]
    InvalidSwapInput,
    #[msg("Missing remaining account")]
    MissingRemainingAccount,
    #[msg("Invalid account data")]
    InvalidAccountData,
    #[msg("CPI swap violated reserved TitanPDA balance")]
    ReservedBalanceViolation,
}

#[macro_export]
macro_rules! debug_logging {
    ($($arg:tt)*) => {
        #[cfg(feature = "swap-logging")]
        msg!($($arg)*)
    };
}

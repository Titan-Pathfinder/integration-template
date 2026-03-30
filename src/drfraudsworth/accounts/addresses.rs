// Hardcoded mainnet addresses for the Dr. Fraudsworth protocol.
//
// All addresses are compile-time constants -- zero network calls needed.

use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

// =============================================================================
// Programs
// =============================================================================

pub const AMM_PROGRAM_ID: Pubkey =
    pubkey!("5JsSAL3kJDUWD4ZveYXYZmgm1eVqueesTZVdAvtZg8cR");

pub const TRANSFER_HOOK_PROGRAM_ID: Pubkey =
    pubkey!("CiQPQrmQh6BPhb9k7dFnsEs5gKPgdrvNKFc5xie5xVGd");

pub const TAX_PROGRAM_ID: Pubkey =
    pubkey!("43fZGRtmEsP7ExnJE1dbTbNjaP1ncvVmMPusSeksWGEj");

pub const EPOCH_PROGRAM_ID: Pubkey =
    pubkey!("4Heqc8QEjJCspHR8y96wgZBnBfbe3Qb8N6JBZMQt9iw2");

pub const STAKING_PROGRAM_ID: Pubkey =
    pubkey!("12b3t1cNiAUoYLiWFEnFa4w6qYxVAiqCWU7KZuzLPYtH");

pub const CONVERSION_VAULT_PROGRAM_ID: Pubkey =
    pubkey!("5uawA6ehYTu69Ggvm3LSK84qFawPKxbWgfngwj15NRJ");

// =============================================================================
// Mints
// =============================================================================

pub const CRIME_MINT: Pubkey =
    pubkey!("cRiMEhAxoDhcEuh3Yf7Z2QkXUXUMKbakhcVqmDsqPXc");

pub const FRAUD_MINT: Pubkey =
    pubkey!("FraUdp6YhtVJYPxC2w255yAbpTsPqd8Bfhy9rC56jau5");

pub const PROFIT_MINT: Pubkey =
    pubkey!("pRoFiTj36haRD5sG2Neqib9KoSrtdYMGrM7SEkZetfR");

pub const NATIVE_MINT: Pubkey =
    pubkey!("So11111111111111111111111111111111111111112");

// =============================================================================
// PDAs
// =============================================================================

pub const EPOCH_STATE_PDA: Pubkey =
    pubkey!("FjJrLcmDjA8FtavGWdhJq3pdirAH889oWXc2bhEAMbDU");

pub const SWAP_AUTHORITY_PDA: Pubkey =
    pubkey!("CoCdbornGtiZ8tLxF5HD2TdGidfgfwbbiDX79BaZGJ2D");

pub const TAX_AUTHORITY_PDA: Pubkey =
    pubkey!("8zijSBnoiGQzwccQkdNuAwbZCieDZsxdn2GgKDErCemQ");

pub const STAKE_POOL_PDA: Pubkey =
    pubkey!("5BdRPPwEDpHEtRgdp4MfywbwmZnrf6u23bXMnG1w8ViN");

pub const ESCROW_VAULT_PDA: Pubkey =
    pubkey!("E68zPDgzMqnycj23g9T74ioHbDdvq3Npj5tT2yPd1SY");

pub const CARNAGE_SOL_VAULT_PDA: Pubkey =
    pubkey!("5988CYMcvJpNtGbtCDnAMxrjrLxRCq3qPME7w2v36aNT");

pub const TREASURY: Pubkey =
    pubkey!("3ihhwLnEJ2duwPSLYxhLbFrdhhxXLcvcrV9rAHqMgzCv");

pub const WSOL_INTERMEDIARY_PDA: Pubkey =
    pubkey!("2HPNULWVVdTcRiAm2DkghLA6frXxA2Nsu4VRu8a4qQ1s");

pub const VAULT_CONFIG_PDA: Pubkey =
    pubkey!("8vFpSBnCVt8dfX57FKrsGwy39TEo1TjVzrj9QYGxCkcD");

pub const VAULT_CRIME: Pubkey =
    pubkey!("Gh9QHMY3J2NGyaHFH2XQCWxedf4G7kBfyu7Jonwn1bHA");

pub const VAULT_FRAUD: Pubkey =
    pubkey!("DLciB9t3qEuRcndGyjRmu1Z34NCwTPvNwbv7eUsFxTZG");

pub const VAULT_PROFIT: Pubkey =
    pubkey!("DBMaWgfUW8WBb8VVvqDFkrMpEkPkCPTcLpSpyzHAiwp3");

// =============================================================================
// Pools
// =============================================================================

pub const CRIME_SOL_POOL: Pubkey =
    pubkey!("ZWUZ3PzGk6bg6g3BS3WdXKbdAecUgZxnruKXQkte7wf");

pub const CRIME_SOL_VAULT_A: Pubkey =
    pubkey!("14rFLiXzXk7aXLnwAz2kwQUjG9vauS84AQLu6LH9idUM");

pub const CRIME_SOL_VAULT_B: Pubkey =
    pubkey!("6s6cprCGxTAYCk9LiwCpCsdHzReW7CLZKqy3ZSCtmV1b");

pub const FRAUD_SOL_POOL: Pubkey =
    pubkey!("AngvViTVGd2zxP8KoFUjGU3TyrQjqeM1idRWiKM8p3mq");

pub const FRAUD_SOL_VAULT_A: Pubkey =
    pubkey!("3sUDyw1k61NSKgn2EA9CaS3FbSZAApGeCRNwNFQPwg8o");

pub const FRAUD_SOL_VAULT_B: Pubkey =
    pubkey!("2nzqXn6FivXjPSgrUGTA58eeVUDjGhvn4QLfhXK1jbjP");

// =============================================================================
// Hook ExtraAccountMetaList PDAs
// =============================================================================

pub const CRIME_HOOK_META: Pubkey =
    pubkey!("CStTzemevJvk8vnjw57Wjzk5EFwN12Nmniz6R7qXWykr");

pub const FRAUD_HOOK_META: Pubkey =
    pubkey!("7QGodnZAYGgastQMXcitcQjraYCMMNDgbp2uL73qjGkd");

pub const PROFIT_HOOK_META: Pubkey =
    pubkey!("J4dubfKw7vnZLhpPfMHqz8PcYWaChugnnSGUgGDzQ9AB");

// =============================================================================
// Standard Programs
// =============================================================================

pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

pub const SYSTEM_PROGRAM_ID: Pubkey =
    pubkey!("11111111111111111111111111111111");

// =============================================================================
// Address Lookup Table
// =============================================================================

/// Protocol-wide ALT containing all program IDs, PDAs, pool addresses,
/// mints, and vaults. Enables v0 transactions for the sell path (25 accounts).
pub const PROTOCOL_ALT: Pubkey =
    pubkey!("7dy5NNvacB8YkZrc3c96vDMDtacXzxVpdPLiC4B7LJ4h");

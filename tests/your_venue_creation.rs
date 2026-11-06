//! ABI fixture using the real SOL3L addresses. Authored from InitMarket,
//! not claimed to be a captured creation transaction.
use solana_pubkey::Pubkey;
use titan_integration_template::{
    ocelot::{self, PROGRAM_ID, USDC},
    trading_venue::{
        protocol::PoolProtocol,
        venue_creation::{ParsedInstruction, PoolCreation},
    },
};

#[test]
fn parses_ocelot_creation_and_ignores_other_instructions() {
    let id = 62030u16;
    let pool = Pubkey::from_str_const("AuvLduWd2g2kmD4kLTGH4c3jf7XqfA9qBUDb4Lmps7kM");
    let mint = Pubkey::from_str_const("BYSvVfnjCh3xzLoCnMbFNnrz331SBMdL7XymyycPSvFv");
    let mut data = ocelot::discriminator("global:init_market").to_vec();
    data.extend(id.to_le_bytes());
    data.extend([0, 2, 3, 1, 0]);
    for fee in [100u16, 30, 10, 0] {
        data.extend(fee.to_le_bytes());
    }
    data.extend(1_000_000u64.to_le_bytes());
    let accounts = vec![
        pool,
        USDC,
        mint,
        Pubkey::find_program_address(&[b"coll", &id.to_le_bytes()], &PROGRAM_ID).0,
        Pubkey::find_program_address(&[b"fee", &id.to_le_bytes()], &PROGRAM_ID).0,
        Pubkey::new_unique(),
        solana_sdk_ids::system_program::ID,
        spl_token::ID,
        solana_sdk_ids::sysvar::rent::ID,
        ocelot::global_bucket(),
    ];
    let ix = ParsedInstruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    };
    assert_eq!(
        ocelot::parse_pool_creations(std::slice::from_ref(&ix)),
        vec![PoolCreation {
            protocol: PoolProtocol::Ocelot,
            pool,
            mints: vec![USDC, mint]
        }]
    );
    let mut unrelated = ix;
    unrelated.data[..8].copy_from_slice(&ocelot::discriminator("global:phoenix_mint_v3"));
    assert!(ocelot::parse_pool_creations(&[unrelated]).is_empty());
}

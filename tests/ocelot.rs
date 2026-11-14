use solana_account::Account;
use solana_pubkey::Pubkey;
use titan_integration_template::{
    ocelot::{
        self,
        codec::{self, Mark, Market, Position},
        instruction,
    },
    trading_venue::{
        FromAccount, QuoteRequest, SwapType, TradingVenue, venue_creation::ParsedInstruction,
    },
};

fn fixture() -> (Pubkey, Account) {
    let id = 62030u16;
    let market =
        Pubkey::find_program_address(&[b"market", &id.to_le_bytes()], &ocelot::PROGRAM_ID).0;
    let mint = Pubkey::find_program_address(&[b"lt2", &id.to_le_bytes()], &ocelot::PROGRAM_ID).0;
    let mut data = vec![0; 399];
    data[..8].copy_from_slice(&Market::DISCRIMINATOR);
    data[104..136].copy_from_slice(ocelot::USDC.as_ref());
    data[136..168].copy_from_slice(mint.as_ref());
    data[232..234].copy_from_slice(&id.to_le_bytes());
    data[235] = 2;
    data[236] = 3;
    data[237] = 1;
    data[239..241].copy_from_slice(&100u16.to_le_bytes());
    data[241..243].copy_from_slice(&30u16.to_le_bytes());
    data[243..245].copy_from_slice(&10u16.to_le_bytes());
    data[247..255].copy_from_slice(&100_000_000u64.to_le_bytes());
    data[317..325].copy_from_slice(&513u64.to_le_bytes());
    (
        market,
        Account {
            owner: ocelot::PROGRAM_ID,
            data,
            ..Default::default()
        },
    )
}
#[test]
fn decoding_pins_owner_discriminator_identity_and_layout() {
    let (address, a) = fixture();
    let m = Market::decode(address, &a).unwrap();
    assert_eq!(m.leverage, 3);
    assert_eq!(m.shares, 100_000_000);
    assert_eq!(m.mint_fee_bps, 10);
    assert_eq!(m.redeem_fee_bps, 30);
    for length in 0..399 {
        let mut short = a.clone();
        short.data.truncate(length);
        assert!(Market::decode(address, &short).is_err());
    }
    let mut bad = a.clone();
    bad.owner = Pubkey::new_unique();
    assert!(Market::decode(address, &bad).is_err());
    bad = a.clone();
    bad.data[0] ^= 1;
    assert!(Market::decode(address, &bad).is_err());
    assert!(Market::decode(Pubkey::new_unique(), &a).is_err());
    bad = a.clone();
    bad.data[237] = 2;
    assert!(Market::decode(address, &bad).is_err());
    bad = a.clone();
    bad.data[317] = 0;
    assert!(Market::decode(address, &bad).is_err());
    bad = a.clone();
    bad.data[325] = 1;
    assert!(Market::decode(address, &bad).is_err());
}
#[test]
fn creation_parser_validates_pdas_venue_and_deduplicates() {
    let (market, a) = fixture();
    let m = Market::decode(market, &a).unwrap();
    let mut data = ocelot::discriminator("global:init_market").to_vec();
    data.extend(m.id.to_le_bytes());
    data.extend([0, 2, 3, 1, 0]);
    data.extend([0; 8]);
    data.extend(1_000_000u64.to_le_bytes());
    let mut accounts = vec![market, ocelot::USDC, m.mint];
    accounts.resize(11, Pubkey::new_unique());
    let ix = ParsedInstruction {
        program_id: ocelot::PROGRAM_ID,
        data,
        accounts,
    };
    let result = ocelot::parse_pool_creations(&[ix.clone(), ix.clone()]);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].mints, vec![ocelot::USDC, m.mint]);
    let mut bad = ix.clone();
    bad.accounts[2] = Pubkey::new_unique();
    assert!(ocelot::parse_pool_creations(&[bad]).is_empty());
    let mut bad = ix.clone();
    bad.program_id = Pubkey::new_unique();
    assert!(ocelot::parse_pool_creations(&[bad]).is_empty());
    for length in 0..31 {
        let mut bad = ix.clone();
        bad.data.truncate(length);
        assert!(ocelot::parse_pool_creations(&[bad]).is_empty());
    }
}
#[test]
fn uninitialized_wrong_pair_and_exact_out_never_quote() {
    let (market, a) = fixture();
    let v = ocelot::OcelotVenue::from_account(&market, &a).unwrap();
    assert!(!v.initialized());
    let mut r = QuoteRequest {
        input_mint: ocelot::USDC,
        output_mint: v.get_token_info()[1].pubkey,
        amount: 0,
        swap_type: SwapType::ExactIn,
    };
    assert!(v.quote(r.clone()).is_err());
    r.swap_type = SwapType::ExactOut;
    assert!(v.direction(&r).is_err());
    r.swap_type = SwapType::ExactIn;
    r.output_mint = ocelot::USDC;
    assert!(v.direction(&r).is_err());
}
#[test]
fn native_price_and_signed_funding_preserve_program_units() {
    let mark = Mark {
        ticks: 123,
        quote_per_lot: 123,
        funding: 7,
        slot: 10,
    };
    let p = Position {
        collateral: 1_000,
        base: 3,
        quote: -300,
        funding: 5,
    };
    assert_eq!(codec::gross(p, mark, 100).unwrap(), 1163);
    let p = Position {
        collateral: 1_000,
        base: -3,
        quote: 300,
        funding: 5,
    };
    assert_eq!(codec::gross(p, mark, 100).unwrap(), 1037);
}
#[test]
fn accrual_prices_accounting_supply_and_caps_fee_at_equity() {
    let (address, a) = fixture();
    let mut m = Market::decode(address, &a).unwrap();
    assert_eq!(
        codec::accrued_liability(&m, 100_000_000, 365 * 24 * 60 * 60).unwrap(),
        1_000_000
    );
    m.liability = 10_000_000;
    assert_eq!(
        codec::accrued_liability(&m, 100_000_000, 365 * 24 * 60 * 60).unwrap(),
        10_900_000
    );
    assert_eq!(
        codec::accrued_liability(&m, 100_000_000, 1000 * 365 * 24 * 60 * 60).unwrap(),
        100_000_000
    );
    m.fee_timestamp = 1;
    assert!(codec::accrued_liability(&m, 100_000_000, 0).is_err());
}
#[test]
fn v3_abi_preserves_full_fill_amounts_and_output_guards() {
    let order = instruction::ioc(42, 1234, true, false);
    assert_eq!(order.len(), 57);
    assert_eq!(order[0], 2);
    assert_eq!(order[1], 0);
    assert_eq!(codec::u64_at(&order, 11).unwrap(), 42);
    assert_eq!(codec::u64_at(&order, 20).unwrap(), 42);
    let mint = instruction::encode(
        true,
        1_000_000,
        999_000,
        &order,
        [10, 9, 10],
        900_000,
        12345,
        7,
    );
    assert_eq!(mint.len(), 148);
    assert_eq!(&mint[..8], &[190, 51, 100, 237, 14, 73, 165, 91]);
    for offset in [20, 32, 44] {
        assert_eq!(codec::u64_at(&mint, offset).unwrap(), 999_000);
    }
    assert_eq!(codec::u64_at(&mint, 124).unwrap(), 900_000);
    assert_eq!(codec::i64_at(&mint, 132).unwrap(), 12345);
    assert_eq!(codec::u64_at(&mint, 140).unwrap(), 7);
    let order = instruction::ioc(7, 1234, false, true);
    assert_eq!(order[55], 128);
    let redeem = instruction::encode(false, 10, 0, &order, [10, 12, 8], 9, 12345, 7);
    assert_eq!(redeem.len(), 112);
    assert_eq!(&redeem[..8], &[207, 0, 207, 230, 122, 44, 120, 250]);
    assert_eq!(codec::u64_at(&redeem, 88).unwrap(), 9);
    assert_eq!(
        instruction::encode(false, 10, 0, &[], [10, 12, 8], 9, 12345, 7).len(),
        55
    );
}

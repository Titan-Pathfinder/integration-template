//! Off-chain helper for turning a quoter [`TradingVenue`] into route instruction
//! inputs.
//!
//! The template mirrors the route-leg shape needed by the program template so
//! you can build and test a route leg for your venue in this repository.
//!
//! ## How a leg fits into the route
//!
//! The route instruction takes `(amount, mints, swaps: Vec<SwapSpecInputV2>)`
//! plus a list of remaining accounts laid out as:
//!
//! ```text
//! [0..mints]        TitanPDA token accounts, one per route mint
//! [mints..2*mints]  mint accounts, aligned with the ATAs
//! [2*mints..]       per-leg venue CPI accounts, concatenated
//! ```
//!
//! Each [`SwapSpecInputV2::n_accounts`] tells the program how many accounts to
//! slice out of that final region for its leg — and that count **includes** the
//! venue program id, which is appended as the leg's last account.
//!
//! ## The two-`Venue`-enums contract
//!
//! [`Venue`] here must match the program template's `Venue` enum
//! (`program-template/.../src/state.rs`) byte-for-byte: same variants, same
//! order. The program template ships an enum-parity test that fails if they
//! drift.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

use crate::trading_venue::{
    QuoteRequest, TradingVenue, error::TradingVenueError, protocol::PoolProtocol,
};

/// Single-byte instruction discriminator for `swap_route_v3`.
pub const SWAP_ROUTE_V3_DISCRIMINATOR: u8 = 42;

/// Weight value meaning "spend the entire available balance on this leg"
/// (weights are in nanos, so `1e9` == 100%).
pub const ROUTE_WEIGHT_ALL: u32 = 1_000_000_000;

/// Route venue selector.
///
/// **Must stay byte-for-byte identical (same variants, same order) to the
/// `Venue` enum in the program template's `state.rs`.** A mismatch can dispatch
/// to the wrong venue. The program template's enum-parity test guards this.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Venue {
    RaydiumAmm,
    // FILL_IN: add your venue variant here, in the SAME position as in
    // `state.rs`. Include any CPI parameters the router must pass to your venue
    // adapter, such as direction flags.
    TemplateVenue { zero_for_one: bool },
}

#[allow(dead_code)]
fn fill_in_route_venue_variant() -> ! {
    todo!("add your route Venue variant in the same position as the program enum")
}

impl Venue {
    /// Borsh-serialized bytes for this variant. Used by the program template's
    /// enum-parity test to cross-check both venue enums.
    pub fn to_borsh_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("Venue serializes infallibly")
    }
}

/// One route leg. Matches the program template's `SwapSpecInputV2` so it
/// Borsh-serializes identically.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SwapSpecInputV2 {
    /// Which on-chain venue executes this leg.
    pub venue: Venue,
    /// Index (into the route's mint list) of the input mint.
    pub from: u8,
    /// Index of the output mint.
    pub to: u8,
    /// Fraction of the available input balance to spend, in nanos (`1e9` = 100%).
    pub weight_nanos: u32,
    /// Number of accounts this leg consumes from the remaining-accounts region,
    /// **including** the trailing venue program id.
    pub n_accounts: u8,
}

/// Map a quoter [`TradingVenue`] and request to its on-chain [`Venue`] variant.
///
/// This must agree with the program template's venue dispatch match. Store any
/// CPI-specific parameters the program needs, such as direction flags, in the
/// returned `Venue` variant.
pub fn protocol_to_venue(
    venue: &dyn TradingVenue,
    request: &QuoteRequest,
) -> Result<Venue, TradingVenueError> {
    match venue.protocol() {
        PoolProtocol::RaydiumAMM => Ok(Venue::RaydiumAmm),
        // FILL_IN: map your PoolProtocol variant to your Venue variant.
        PoolProtocol::YourPoolProtocol => {
            let _ = (venue, request);
            todo!("map YourPoolProtocol to your Venue variant")
        }
    }
}

/// Build a single route leg for `swap_route_v3` from a quoter venue.
///
/// Builds a route leg from the venue's swap instruction:
///
/// 1. clears the TitanPDA signer flag because it signs through PDA seeds,
/// 2. appends the venue program id as the leg's final (read-only) account, and
/// 3. records `n_accounts = venue accounts + 1`.
///
/// Returns the leg's [`SwapSpecInputV2`] and the ordered [`AccountMeta`]s
/// (length == `n_accounts`) to append to the route's remaining accounts.
///
/// `from`/`to` are the input/output mint indices within the route; `titan_pda`
/// is the PDA that custodies funds mid-route and is passed to the venue as the
/// swap's user/authority.
pub fn build_swap_leg(
    venue: &dyn TradingVenue,
    request: &QuoteRequest,
    titan_pda: Pubkey,
    from: u8,
    to: u8,
    weight_nanos: u32,
) -> Result<(SwapSpecInputV2, Vec<AccountMeta>), TradingVenueError> {
    let swap_ix = venue.generate_swap_instruction(request.clone(), titan_pda)?;
    let accounts = assemble_leg_accounts(&swap_ix, titan_pda, venue.program_id());

    let spec = SwapSpecInputV2 {
        venue: protocol_to_venue(venue, request)?,
        from,
        to,
        weight_nanos,
        n_accounts: accounts.len() as u8,
    };

    Ok((spec, accounts))
}

/// Turn a venue's raw swap instruction into the leg's remaining-accounts list:
/// pass every account through unchanged except TitanPDA (whose signer flag is
/// cleared), then append `venue_program_id` as the read-only trailing account.
fn assemble_leg_accounts(
    swap_ix: &Instruction,
    titan_pda: Pubkey,
    venue_program_id: Pubkey,
) -> Vec<AccountMeta> {
    let mut accounts: Vec<AccountMeta> = swap_ix
        .accounts
        .iter()
        .cloned()
        .map(|acc| {
            if acc.pubkey == titan_pda {
                AccountMeta {
                    is_signer: false,
                    ..acc
                }
            } else {
                acc
            }
        })
        .collect();

    accounts.push(AccountMeta::new_readonly(venue_program_id, false));
    accounts
}

/// Encode the full `swap_route_v3` instruction data: the single-byte
/// discriminator followed by the Borsh serialization of `(amount, mints, swaps)`.
///
/// This is the wire format expected by the program template.
pub fn encode_swap_route_v3_data(amount: u64, mints: u8, swaps: &[SwapSpecInputV2]) -> Vec<u8> {
    let mut data = vec![SWAP_ROUTE_V3_DISCRIMINATOR];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(mints);
    // Borsh encodes a Vec as a u32 little-endian length followed by its elements.
    data.extend_from_slice(&(swaps.len() as u32).to_le_bytes());
    for swap in swaps {
        swap.serialize(&mut data)
            .expect("borsh serialize into Vec is infallible");
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_caching::AccountsCache;
    use crate::trading_venue::{QuoteResult, SwapType, token_info::TokenInfo};
    use async_trait::async_trait;

    const VENUE_PID: Pubkey = Pubkey::new_from_array([7u8; 32]);

    /// Minimal venue whose `generate_swap_instruction` returns a canned
    /// instruction: `[titan_pda (signer, writable), other (writable)]`.
    struct MockVenue {
        titan_pda: Pubkey,
        other: Pubkey,
        protocol: PoolProtocol,
        token_info: Vec<TokenInfo>,
    }

    #[async_trait]
    impl TradingVenue for MockVenue {
        fn initialized(&self) -> bool {
            true
        }
        fn program_id(&self) -> Pubkey {
            VENUE_PID
        }
        fn program_dependencies(&self) -> Vec<Pubkey> {
            vec![VENUE_PID]
        }
        fn market_id(&self) -> Pubkey {
            Pubkey::default()
        }
        fn get_token_info(&self) -> &[TokenInfo] {
            &self.token_info
        }
        fn protocol(&self) -> PoolProtocol {
            self.protocol
        }
        fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
            Ok(vec![])
        }
        async fn update_state(
            &mut self,
            _cache: &dyn AccountsCache,
        ) -> Result<(), TradingVenueError> {
            Ok(())
        }
        fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
            Ok(QuoteResult {
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                amount: request.amount,
                expected_output: request.amount,
                not_enough_liquidity: false,
                price: 1.0,
            })
        }
        fn generate_swap_instruction(
            &self,
            _request: QuoteRequest,
            _user: Pubkey,
        ) -> Result<Instruction, TradingVenueError> {
            Ok(Instruction {
                program_id: VENUE_PID,
                accounts: vec![
                    AccountMeta::new(self.titan_pda, true),
                    AccountMeta::new(self.other, false),
                ],
                data: vec![],
            })
        }
    }

    fn request() -> QuoteRequest {
        QuoteRequest {
            input_mint: Pubkey::new_from_array([1u8; 32]),
            output_mint: Pubkey::new_from_array([2u8; 32]),
            amount: 1_000,
            swap_type: SwapType::ExactIn,
        }
    }

    fn mock_venue(protocol: PoolProtocol, token_info: Vec<TokenInfo>) -> MockVenue {
        MockVenue {
            titan_pda: Pubkey::new_from_array([9u8; 32]),
            other: Pubkey::new_from_array([8u8; 32]),
            protocol,
            token_info,
        }
    }

    #[test]
    fn protocol_maps_to_venue() {
        let request = request();
        let raydium = mock_venue(PoolProtocol::RaydiumAMM, vec![]);
        assert_eq!(
            protocol_to_venue(&raydium, &request).unwrap(),
            Venue::RaydiumAmm
        );
    }

    #[test]
    fn build_swap_leg_clears_titan_pda_signer_and_appends_program_id() {
        let titan_pda = Pubkey::new_from_array([9u8; 32]);
        let venue = MockVenue {
            titan_pda,
            other: Pubkey::new_from_array([8u8; 32]),
            protocol: PoolProtocol::RaydiumAMM,
            token_info: vec![],
        };

        let (spec, accounts) =
            build_swap_leg(&venue, &request(), titan_pda, 0, 1, ROUTE_WEIGHT_ALL).unwrap();

        // Two venue accounts + the appended program id.
        assert_eq!(accounts.len(), 3);
        assert_eq!(spec.n_accounts, 3);
        assert_eq!(spec.venue, Venue::RaydiumAmm);
        assert_eq!((spec.from, spec.to), (0, 1));

        // TitanPDA must no longer be marked a signer.
        let titan_pda_meta = accounts.iter().find(|a| a.pubkey == titan_pda).unwrap();
        assert!(!titan_pda_meta.is_signer);

        // Last account is the read-only venue program id.
        let last = accounts.last().unwrap();
        assert_eq!(last.pubkey, VENUE_PID);
        assert!(!last.is_signer && !last.is_writable);
    }

    #[test]
    fn encodes_instruction_data_like_anchor() {
        let spec = SwapSpecInputV2 {
            venue: Venue::RaydiumAmm,
            from: 0,
            to: 1,
            weight_nanos: ROUTE_WEIGHT_ALL,
            n_accounts: 3,
        };
        let data = encode_swap_route_v3_data(5_000, 2, &[spec]);

        let mut expected = vec![SWAP_ROUTE_V3_DISCRIMINATOR];
        expected.extend_from_slice(&5_000u64.to_le_bytes()); // amount
        expected.push(2); // mints
        expected.extend_from_slice(&1u32.to_le_bytes()); // swaps len
        expected.push(0); // Venue::RaydiumAmm discriminant
        expected.push(0); // from
        expected.push(1); // to
        expected.extend_from_slice(&ROUTE_WEIGHT_ALL.to_le_bytes());
        expected.push(3); // n_accounts
        assert_eq!(data, expected);
    }

    #[test]
    fn venue_borsh_bytes_are_stable() {
        assert_eq!(Venue::RaydiumAmm.to_borsh_bytes(), vec![0]);
        assert_eq!(
            Venue::TemplateVenue {
                zero_for_one: false,
            }
            .to_borsh_bytes(),
            vec![1, 0]
        );
        assert_eq!(
            Venue::TemplateVenue { zero_for_one: true }.to_borsh_bytes(),
            vec![1, 1]
        );
    }
}

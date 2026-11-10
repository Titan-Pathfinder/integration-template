//! `local-stand setup` — build the local-validator stand for the Coffer
//! venue: local mints, the Titan router PDA, and one real Coffer pool per
//! matrix configuration (created, seeded and configured with the local wallet
//! as pool admin), then write `scripts/local-stand/stand.json`.
//!
//! Run through `scripts/local-stand/up.sh`, which starts the validator with
//! the production Coffer ELF, the router built from `program-template`
//! and the cloned `CofferPoolConfig`.

use std::str::FromStr;

use solana_client::rpc_client::RpcClient;
use solana_instruction::Instruction;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::{Keypair, Signer, read_keypair_file};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;
use spl_token_2022::extension::ExtensionType;
use spl_token_2022::state::Mint as Mint2022;

use titan_integration_template::local_stand::*;

const BONK_DECIMALS: u8 = 5;
const USDC_DECIMALS: u8 = 6;
const SOL_DECIMALS: u8 = 9;
const T22_DECIMALS: u8 = 6;

/// A case of the matrix: BONK is always slot 0 and USDC slot 1 for 2-token
/// pools; the 4-token pool is SOL/BONK/USDC/MEME22.
struct Case {
    id: &'static str,
    description: &'static str,
    weights: &'static [u64],
    tokens: &'static [&'static str],
    swap_fee_rate: u32,
    /// selloff params per slot (`SelloffParams::OFF` for none)
    selloff: &'static [SelloffParams],
    inactive: &'static [u8],
    swaps_enabled: bool,
    pool_enabled: bool,
    dynamic: bool,
}

const H: u32 = 3_600;
const OFF: SelloffParams = SelloffParams::OFF;
// (cap, period, threshold, low, mid, high, kink)
const CAP_ONLY: SelloffParams = SelloffParams::new(1_000, H, 0, 0, 0, 0, 0);
const B: SelloffParams = SelloffParams::new(1_000, H, 0, 0, 5_000, 10_000, 50);
const C: SelloffParams = SelloffParams::new(1_000, H, 8_000, 0, 0, 10_000, 0);
const D: SelloffParams = SelloffParams::new(1_000, H, 8_000, 10_000, 10_000, 10_000, 0);
const E: SelloffParams = SelloffParams::new(1_000, H, 9_990, 0, 5_000, 10_000, 0);
const E2: SelloffParams = SelloffParams::new(1_000, H, 9_899, 0, 5_000, 10_000, 99);
const F: SelloffParams = SelloffParams::new(1_000, H, 5_000, 500, 500, 500, 60);
const G1: SelloffParams = SelloffParams::new(1_000, H, 0, 0, 2_000, 10_000, 1);
const G99: SelloffParams = SelloffParams::new(1_000, H, 0, 0, 2_000, 10_000, 99);
const H100: SelloffParams = SelloffParams::new(10_000, H, 5_000, 0, 2_500, 5_000, 75);
const H1: SelloffParams = SelloffParams::new(100, H, 5_000, 0, 2_500, 5_000, 75);
const SHORT: SelloffParams = SelloffParams::new(1_000, 20, 5_000, 0, 2_000, 5_000, 80);
const J: SelloffParams = SelloffParams::new(0, 0, 5_000, 100, 500, 3_000, 75);
const M_BONK: SelloffParams = SelloffParams::new(1_000, H, 5_000, 0, 1_000, 3_000, 70);
const M_T22: SelloffParams = SelloffParams::new(500, H, 8_000, 0, 0, 5_000, 0);
/// Range-manager scenarios: a 30 s window with a surge curve (real-time
/// rotations under vb/weight moves), and a moderate curve for the leverage case.
const RM_SHORT: SelloffParams = SelloffParams::new(1_000, 30, 5_000, 0, 2_000, 5_000, 80);
const RM_LEV: SelloffParams = SelloffParams::new(1_000, H, 5_000, 0, 1_000, 3_000, 70);

const TWO: &[&str] = &["BONK", "USDC"];
const FOUR: &[&str] = &["SOLX", "BONK", "USDC", "MEME22"];

/// The configuration matrix. Static pools are read by the shared suite and
/// the route simulation; `dynamic` pools are mutated by the sequence tests.
const CASES: &[Case] = &[
    Case {
        id: "a_cap_only",
        description: "cap 10%, surge off (slope_high = 0)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[CAP_ONLY, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "b_thr0_kink50",
        description: "cap 10%, threshold 0, 0/50/100%, kink 50",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "c_thr80_nokink",
        description: "cap 10%, threshold 80%, 0/0/100%, no kink",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[C, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "d_thr80_full_fee",
        description: "cap 10%, threshold 80%, 100/100/100% (all output above threshold taken)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[D, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "e_thr999_nokink",
        description: "cap 10%, threshold 99.9%, 0/50/100% (kink 99 is rejected on-chain: kink must exceed the threshold)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[E, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "e2_thr9899_kink99",
        description: "cap 10%, threshold 98.99%, 0/50/100%, kink 99 (the closest legal kink-99 config)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[E2, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "f_flat",
        description: "cap 10%, threshold 50%, flat 5/5/5%, kink 60 (kink must exceed the threshold on-chain)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[F, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "g_kink1",
        description: "cap 10%, threshold 0, 0/20/100%, kink 1",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[G1, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "g_kink99",
        description: "cap 10%, threshold 0, 0/20/100%, kink 99 (kink 100 is rejected on-chain)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[G99, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "h_cap100",
        description: "cap 100%, threshold 50%, 0/25/50%, kink 75",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[H100, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "h_cap1",
        description: "cap 1%, threshold 50%, 0/25/50%, kink 75",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[H1, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "i_short_period",
        description: "cap 10%, period 20 s, threshold 50%, 0/20/50%, kink 80",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[SHORT, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "j_surge_no_cap",
        description: "surge curve configured, cap disabled (max_selloff_pct = 0): surge must be inert",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[J, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "k_inactive",
        description: "config b + BONK deactivated (set_token_active false)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[0],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "l_swaps_off",
        description: "config a + swaps disabled",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[CAP_ONLY, OFF],
        inactive: &[],
        swaps_enabled: false,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "l_pool_off",
        description: "config a + pool disabled (protocol admin)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[CAP_ONLY, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: false,
        dynamic: false,
    },
    Case {
        id: "w8020_b",
        description: "80/20 BONK/USDC, config b",
        weights: &[8_000, 2_000],
        tokens: TWO,
        swap_fee_rate: 5_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "w8020_c",
        description: "80/20 BONK/USDC, config c",
        weights: &[8_000, 2_000],
        tokens: TWO,
        swap_fee_rate: 5_000,
        selloff: &[C, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    Case {
        id: "m_four_token",
        description: "SOL/BONK/USDC/MEME22 25% each; BONK cap 10% thr 50% 0/10/30% kink 70; MEME22 (Token-2022) cap 5% thr 80% 0/0/50%",
        weights: &[2_500, 2_500, 2_500, 2_500],
        tokens: FOUR,
        swap_fee_rate: 2_500,
        selloff: &[OFF, M_BONK, OFF, M_T22],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: false,
    },
    // dynamic pools (state is mutated by the sequence tests)
    Case {
        id: "dyn_b",
        description: "dynamic: config b",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "dyn_c",
        description: "dynamic: config c",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[C, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "dyn_d",
        description: "dynamic: config d (100% fee above threshold)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[D, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "dyn_short",
        description: "dynamic: 20 s period, real-time rotation",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[SHORT, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "dyn_8020_c",
        description: "dynamic: 80/20, config c",
        weights: &[8_000, 2_000],
        tokens: TWO,
        swap_fee_rate: 5_000,
        selloff: &[C, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "dyn_toggle",
        description: "dynamic: config b, deactivate / swaps / pool toggles",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "dyn_router",
        description: "dynamic: config b, chunked sells through the router",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    // range-manager / admin scenarios (tests/local_stand_range_manager.rs)
    Case {
        id: "rm_short",
        description: "range manager: 30 s window, cap 10%, thr 50%, 0/20/50%, kink 80 — vb moves mid-window, rotation, no-refresh execution",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[RM_SHORT, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "rm_w_surge",
        description: "range manager: weight moves 50/50 -> 55/45 -> 45/55 with config b (surge)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "rm_w_cap",
        description: "range manager: weight moves 50/50 -> 55/45 -> 45/55 with the cap only (no surge)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[CAP_ONLY, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "rm_lev",
        description: "range manager: USDC vb pushed to 16x so the LP balance caps output before the window, then back",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[RM_LEV, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "rm_cfg",
        description: "admin: set_max_selloff reconfigured between quotes (cap raised / lowered / curve changed / disabled / re-enabled)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
    Case {
        id: "rm_liq",
        description: "admin: add_liquidity / remove_liquidity between quotes (window rescale)",
        weights: &[5_000, 5_000],
        tokens: TWO,
        swap_fee_rate: 3_000,
        selloff: &[B, OFF],
        inactive: &[],
        swaps_enabled: true,
        pool_enabled: true,
        dynamic: true,
    },
];

/// Virtual / actual balances per token name (atoms). BONK ≈ $0.00002,
/// USDC $1, SOLX $200, MEME22 $1; equal-value slots in the 4-token pool.
fn balances(name: &str, weight: u64, total_weight: u64) -> (u64, u64) {
    // value per slot: $50k for 2-token pools (scaled by weight share), $200k
    // per slot in the 4-token pool
    let usd = if total_weight == 10_000 && weight == 2_500 {
        200_000.0
    } else {
        100_000.0 * weight as f64 / total_weight as f64
    };
    let vb = match name {
        "BONK" => usd / 0.00002 * 1e5,
        "USDC" => usd * 1e6,
        "SOLX" => usd / 200.0 * 1e9,
        "MEME22" => usd * 1e6,
        _ => unreachable!(),
    } as u64;
    (vb, vb / 2)
}

struct Ctx {
    rpc: RpcClient,
    wallet: Keypair,
}

impl Ctx {
    fn send(&self, ixs: &[Instruction], extra_signers: &[&Keypair]) -> String {
        let bh = self.rpc.get_latest_blockhash().unwrap();
        let mut signers: Vec<&Keypair> = vec![&self.wallet];
        signers.extend_from_slice(extra_signers);
        let tx = Transaction::new_signed_with_payer(ixs, Some(&self.wallet.pubkey()), &signers, bh);
        match self.rpc.send_and_confirm_transaction(&tx) {
            Ok(sig) => sig.to_string(),
            Err(e) => panic!("transaction failed: {e:#?}"),
        }
    }
}

fn create_spl_mint(ctx: &Ctx, decimals: u8) -> Pubkey {
    let mint = Keypair::new();
    let rent = ctx
        .rpc
        .get_minimum_balance_for_rent_exemption(spl_token::state::Mint::LEN)
        .unwrap();
    ctx.send(
        &[
            system_instruction::create_account(
                &ctx.wallet.pubkey(),
                &mint.pubkey(),
                rent,
                spl_token::state::Mint::LEN as u64,
                &spl_token::ID,
            ),
            spl_token::instruction::initialize_mint2(
                &spl_token::ID,
                &mint.pubkey(),
                &ctx.wallet.pubkey(),
                None,
                decimals,
            )
            .unwrap(),
        ],
        &[&mint],
    );
    mint.pubkey()
}

/// Token-2022 mint with only the MetadataPointer + TokenMetadata extensions.
fn create_t22_metadata_mint(ctx: &Ctx, decimals: u8, name: &str, symbol: &str) -> Pubkey {
    let mint = Keypair::new();
    let space =
        ExtensionType::try_calculate_account_len::<Mint2022>(&[ExtensionType::MetadataPointer])
            .unwrap();
    let rent = ctx
        .rpc
        .get_minimum_balance_for_rent_exemption(space + 256)
        .unwrap();
    let auth = ctx.wallet.pubkey();
    ctx.send(
        &[
            system_instruction::create_account(
                &auth,
                &mint.pubkey(),
                rent,
                space as u64,
                &spl_token_2022::ID,
            ),
            spl_token_2022::extension::metadata_pointer::instruction::initialize(
                &spl_token_2022::ID,
                &mint.pubkey(),
                Some(auth),
                Some(mint.pubkey()),
            )
            .unwrap(),
            spl_token_2022::instruction::initialize_mint2(
                &spl_token_2022::ID,
                &mint.pubkey(),
                &auth,
                None,
                decimals,
            )
            .unwrap(),
            spl_token_metadata_interface::instruction::initialize(
                &spl_token_2022::ID,
                &mint.pubkey(),
                &auth,
                &mint.pubkey(),
                &auth,
                name.to_string(),
                symbol.to_string(),
                "https://example.invalid/meme22.json".to_string(),
            ),
        ],
        &[&mint],
    );
    mint.pubkey()
}

fn mint_to_wallet(ctx: &Ctx, mint: Pubkey, tp: Pubkey, amount: u64) {
    let owner = ctx.wallet.pubkey();
    let create =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &owner, &owner, &mint, &tp,
        );
    let to = ata(&owner, &mint, &tp);
    let mint_ix = if tp == spl_token::ID {
        spl_token::instruction::mint_to(&tp, &mint, &to, &owner, &[], amount).unwrap()
    } else {
        spl_token_2022::instruction::mint_to(&tp, &mint, &to, &owner, &[], amount).unwrap()
    };
    ctx.send(&[create, mint_ix], &[]);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("setup") {
        eprintln!("usage: local-stand setup   (env: LOCAL_STAND_RPC, LOCAL_STAND_WALLET)");
        std::process::exit(2);
    }
    let rpc_url = std::env::var("LOCAL_STAND_RPC").unwrap_or_else(|_| LOCAL_RPC.to_string());
    let wallet_path = std::env::var("LOCAL_STAND_WALLET")
        .unwrap_or_else(|_| format!("{}/.config/solana/id.json", std::env::var("HOME").unwrap()));
    let ctx = Ctx {
        rpc: RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed()),
        wallet: read_keypair_file(&wallet_path).expect("wallet keypair"),
    };
    let me = ctx.wallet.pubkey();
    println!("wallet {me}");
    if ctx.rpc.get_balance(&me).unwrap() < 50_000_000_000 {
        let sig = ctx.rpc.request_airdrop(&me, 500_000_000_000).unwrap();
        ctx.rpc
            .confirm_transaction_with_spinner(
                &sig,
                &ctx.rpc.get_latest_blockhash().unwrap(),
                CommitmentConfig::confirmed(),
            )
            .unwrap();
    }
    assert!(
        ctx.rpc.get_account(&LOCAL_CONFIG).is_ok(),
        "CofferPoolConfig {LOCAL_CONFIG} not loaded into the validator"
    );

    // Mints.
    let bonk = create_spl_mint(&ctx, BONK_DECIMALS);
    let usdc = create_spl_mint(&ctx, USDC_DECIMALS);
    let solx = create_spl_mint(&ctx, SOL_DECIMALS);
    let meme22 = create_t22_metadata_mint(&ctx, T22_DECIMALS, "Meme22", "MEME22");
    let mints = vec![
        StandMint {
            name: "BONK".into(),
            address: bonk.to_string(),
            decimals: BONK_DECIMALS,
            token_2022: false,
        },
        StandMint {
            name: "USDC".into(),
            address: usdc.to_string(),
            decimals: USDC_DECIMALS,
            token_2022: false,
        },
        StandMint {
            name: "SOLX".into(),
            address: solx.to_string(),
            decimals: SOL_DECIMALS,
            token_2022: false,
        },
        StandMint {
            name: "MEME22".into(),
            address: meme22.to_string(),
            decimals: T22_DECIMALS,
            token_2022: true,
        },
    ];
    let lookup = |name: &str| -> (Pubkey, Pubkey) {
        let m = mints.iter().find(|m| m.name == name).unwrap();
        (
            Pubkey::from_str(&m.address).unwrap(),
            if m.token_2022 {
                spl_token_2022::ID
            } else {
                spl_token::ID
            },
        )
    };
    for m in &mints {
        let (mint, tp) = lookup(&m.name);
        mint_to_wallet(&ctx, mint, tp, 4_000_000_000_000_000_000); // 4e18 atoms of each
        println!(
            "mint {} {} (decimals {}, t22 {})",
            m.name, m.address, m.decimals, m.token_2022
        );
    }

    // Router TitanPDA.
    let pda = titan_pda();
    if ctx.rpc.get_account(&pda).is_err() {
        ctx.send(&[router_initialize_ix(me)], &[]);
    }
    println!("router {ROUTER_PROGRAM_ID} titan_pda {pda}");

    // Pools.
    let base_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        * 1_000;
    let mut pools = Vec::new();
    for (i, case) in CASES.iter().enumerate() {
        let token_keys: Vec<(Pubkey, Pubkey)> = case.tokens.iter().map(|n| lookup(n)).collect();
        let total_w: u64 = case.weights.iter().sum();
        let (vbs, abs): (Vec<u64>, Vec<u64>) = case
            .tokens
            .iter()
            .zip(case.weights)
            .map(|(n, w)| balances(n, *w, total_w))
            .unzip();
        let pool_id = base_id + i as u64;
        let (pool, init) = initialize_pool_ix(
            LOCAL_CONFIG,
            me,
            pool_id,
            &token_keys,
            case.weights,
            &vbs,
            case.swap_fee_rate,
        );
        ctx.send(&[init], &[]);

        let mut seed = Vec::new();
        for (mint, tp) in &token_keys {
            seed.push(spl_associated_token_account::instruction::create_associated_token_account_idempotent(&me, &pool, mint, tp));
        }
        let bpt = bpt_mint(&pool);
        seed.push(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &me,
                &me,
                &bpt,
                &spl_token::ID,
            ),
        );
        seed.push(add_liquidity_ix(pool, me, &token_keys, &abs, 0));
        ctx.send(&seed, &[]);

        let mut cfg = vec![set_max_selloff_ix(pool, me, case.selloff)];
        for &slot in case.inactive {
            cfg.push(set_token_active_ix(LOCAL_CONFIG, pool, me, slot, false));
        }
        if !case.swaps_enabled {
            cfg.push(set_swaps_enabled_ix(LOCAL_CONFIG, pool, me, false));
        }
        if !case.pool_enabled {
            cfg.push(set_pool_enabled_ix(LOCAL_CONFIG, pool, me, false));
        }
        ctx.send(&cfg, &[]);

        println!("pool {:<20} {pool}  {}", case.id, case.description);
        pools.push(StandPool {
            case: case.id.to_string(),
            description: case.description.to_string(),
            address: pool.to_string(),
            tokens: case.tokens.iter().map(|s| s.to_string()).collect(),
            weights: case.weights.to_vec(),
            swap_fee_rate: case.swap_fee_rate,
            selloff: case.selloff.to_vec(),
            inactive_tokens: case.inactive.to_vec(),
            swaps_enabled: case.swaps_enabled,
            pool_enabled: case.pool_enabled,
            dynamic: case.dynamic,
        });
    }

    let manifest = StandManifest {
        rpc: rpc_url,
        wallet: me.to_string(),
        config: LOCAL_CONFIG.to_string(),
        router: ROUTER_PROGRAM_ID.to_string(),
        titan_pda: pda.to_string(),
        created_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        mints,
        pools,
    };
    manifest.save();
    println!("wrote {}", StandManifest::path().display());
}

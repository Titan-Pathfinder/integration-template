use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::{invoke, invoke_signed};
use anchor_spl::associated_token::spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use anchor_spl::token::spl_token;
use anchor_spl::token::{transfer, Transfer};
use anchor_spl::token_2022::{close_account, CloseAccount, ID as TOKEN_PROGRAM_2022_ID};
use anchor_spl::token_interface::{transfer_checked, TransferChecked};

use crate::error::TemplateError;
use crate::instructions::venues::raydium_amm;
use crate::state::{SwapSpecInputV2, TitanPda, Venue, MAX_MINTS, MAX_SWAPS};

#[derive(Accounts)]
#[instruction(amount: u64, mints: u8, swaps: Vec<SwapSpecInputV2>)]
pub struct SwapRouteV3<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [TitanPda::SEED], bump = titan_pda.bump)]
    pub titan_pda: Box<Account<'info, TitanPda>>,
    /// CHECK: user's input token account; SPL ownership is checked by token CPI.
    #[account(mut)]
    pub input_token_account: UncheckedAccount<'info>,
    /// CHECK: user's output token account.
    #[account(mut)]
    pub output_token_account: UncheckedAccount<'info>,
    /// CHECK: standard SPL Token program.
    pub token_program: AccountInfo<'info>,
    /// CHECK: Token-2022 program.
    pub token_2022_program: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: associated token program.
    pub associated_token_program: AccountInfo<'info>,
    /// CHECK: optional token ledger/input amount source.
    pub token_ledger: Option<AccountInfo<'info>>,
    /// CHECK: reserved optional route account; pass this program id when unused.
    pub reserved_optional_account_0: Option<AccountInfo<'info>>,
    /// CHECK: reserved optional route account; pass this program id when unused.
    pub reserved_optional_account_1: Option<AccountInfo<'info>>,
}

/// Remaining account layout:
/// [0..mints] TitanPDA token accounts, one for each route mint.
/// [mints..2*mints] mint accounts aligned with the ATAs above.
/// [2*mints..N] venue CPI accounts for each swap leg.
///
/// Each swap leg must append the venue program id as the final account for that leg.
/// `n_accounts` includes that program id. The dispatcher passes all leg accounts to
/// `invoke_signed`, but omits the final program account from the venue's `AccountMeta`s.
impl SwapRouteV3<'_> {
    pub fn execute<'info>(
        ctx: Context<'_, '_, 'info, 'info, SwapRouteV3<'info>>,
        amount: u64,
        mints: u8,
        swaps: Vec<SwapSpecInputV2>,
    ) -> Result<()> {
        require!(
            mints > 1 && !swaps.is_empty(),
            TemplateError::InvalidSwapInput
        );
        require!(swaps.len() <= MAX_SWAPS, TemplateError::InvalidSwapInput);

        let mints_count = mints as usize;
        require!(mints_count <= MAX_MINTS, TemplateError::InvalidSwapInput);
        require!(
            ctx.remaining_accounts.len() >= 2 * mints_count,
            TemplateError::MissingRemainingAccount
        );
        require!(swaps[0].from == 0, TemplateError::InvalidSwapInput);

        for swap in swaps.iter() {
            require!(
                swap.from < mints && swap.to < mints,
                TemplateError::InvalidSwapInput
            );
        }

        let ledger_amount = read_token_ledger(
            &ctx.accounts.token_ledger,
            &ctx.accounts.input_token_account.to_account_info(),
        )?;

        let bump = [ctx.accounts.titan_pda.bump];
        let signer_seeds: &[&[&[u8]]] = &[&[TitanPda::SEED, &bump]];
        let titan_pda_key = ctx.accounts.titan_pda.key();

        let input_mint = ctx
            .remaining_accounts
            .get(mints_count)
            .ok_or(TemplateError::MissingRemainingAccount)?;
        let output_ata_index = swaps[swaps.len() - 1].to as usize;
        let output_mint = ctx
            .remaining_accounts
            .get(mints_count + output_ata_index)
            .ok_or(TemplateError::MissingRemainingAccount)?;

        let input_mint_decimals = extract_mint_decimals(input_mint)?;
        let output_mint_decimals = extract_mint_decimals(output_mint)?;

        let mut created_atas: u16 = 0;
        for i in 0..mints_count {
            let ata = ctx
                .remaining_accounts
                .get(i)
                .ok_or(TemplateError::MissingRemainingAccount)?;

            if !account_exists(ata) {
                let mint_account = ctx
                    .remaining_accounts
                    .get(mints_count + i)
                    .ok_or(TemplateError::MissingRemainingAccount)?;
                let token_program = get_token_program_for_mint(
                    mint_account,
                    &ctx.accounts.token_program,
                    &ctx.accounts.token_2022_program,
                );

                create_titan_pda_ata(
                    &ctx.accounts.payer.to_account_info(),
                    &ctx.accounts.titan_pda.to_account_info(),
                    mint_account,
                    ata,
                    token_program,
                    &ctx.accounts.system_program.to_account_info(),
                )?;
                created_atas |= 1 << i;
            }
        }

        let mut titan_pda_ata_bitmap = TitanPdaBitmap::new();
        let mut titan_pda_ata_balances = Vec::new();
        let mut reserved_balances = [0u64; MAX_MINTS];
        for (i, account) in ctx.remaining_accounts.iter().enumerate() {
            if i < mints_count {
                require!(
                    is_titan_pda_token_account(account, &titan_pda_key),
                    TemplateError::InvalidAccountData
                );
                let balance = extract_token_amount(account)?;
                reserved_balances[i] = balance;
                titan_pda_ata_bitmap.set(i);
                titan_pda_ata_balances.push(balance);
            } else if is_titan_pda_token_account(account, &titan_pda_key) {
                titan_pda_ata_bitmap.set(i);
                titan_pda_ata_balances.push(extract_token_amount(account)?);
            }
        }

        let effective_amount = match ledger_amount {
            Some(stored) => {
                let current = extract_token_amount(&ctx.accounts.input_token_account)?;
                let delta = current
                    .checked_sub(stored)
                    .ok_or(TemplateError::InvalidAccountData)?;
                require!(delta > 0, TemplateError::InvalidAccountData);
                delta
            }
            None => amount,
        };

        let first_titan_pda_ata = ctx
            .remaining_accounts
            .first()
            .ok_or(TemplateError::MissingRemainingAccount)?;
        let input_token_program = get_token_program_for_mint(
            input_mint,
            &ctx.accounts.token_program,
            &ctx.accounts.token_2022_program,
        );
        let input_is_native = is_native_mint(input_mint);
        let mut created_wsol_input = false;

        if ledger_amount.is_none() && input_is_native {
            created_wsol_input = setup_wsol_input_account(
                &ctx.accounts.payer.to_account_info(),
                &ctx.accounts.user.to_account_info(),
                &ctx.accounts.input_token_account,
                input_mint,
                input_token_program,
                &ctx.accounts.system_program.to_account_info(),
                effective_amount,
            )?;
        }

        transfer_tokens(
            input_token_program,
            &ctx.accounts.input_token_account,
            &first_titan_pda_ata.to_account_info(),
            &ctx.accounts.user.to_account_info(),
            Some(input_mint),
            effective_amount,
            input_mint_decimals,
            None,
        )?;

        if created_wsol_input {
            close_token_account(
                &ctx.accounts.input_token_account,
                &ctx.accounts.payer.to_account_info(),
                &ctx.accounts.user.to_account_info(),
                input_token_program,
                None,
            )?;
        }

        let mut remaining_account_index = 2 * mints_count;
        for swap in swaps.iter() {
            let account_increment = swap.n_accounts as usize;
            require!(account_increment > 0, TemplateError::InvalidSwapInput);
            require!(
                remaining_account_index + account_increment <= ctx.remaining_accounts.len(),
                TemplateError::MissingRemainingAccount
            );

            let input_token_account = ctx
                .remaining_accounts
                .get(swap.from as usize)
                .ok_or(TemplateError::MissingRemainingAccount)?;

            let input_token_balance = extract_token_amount(input_token_account)?;
            let available_balance =
                input_token_balance.saturating_sub(reserved_balances[swap.from as usize]);
            let weight = swap.weight_nanos.min(1_000_000_000) as u128;
            let input_amount = (available_balance as u128)
                .saturating_mul(weight)
                .saturating_div(1_000_000_000u128) as u64;

            if input_amount > 0 {
                let leg_accounts = &ctx.remaining_accounts
                    [remaining_account_index..remaining_account_index + account_increment];
                let account_metas = leg_accounts[..account_increment - 1]
                    .iter()
                    .map(|account| AccountMeta {
                        pubkey: *account.key,
                        is_signer: account.key == &titan_pda_key || account.is_signer,
                        is_writable: account.is_writable,
                    })
                    .collect::<Vec<_>>();

                perform_cpi_swap(
                    input_amount,
                    swap,
                    &account_metas,
                    leg_accounts,
                    signer_seeds,
                )?;
            }

            remaining_account_index += account_increment;
        }

        let titan_pda_output_token_account = ctx
            .remaining_accounts
            .get(output_ata_index)
            .ok_or(TemplateError::MissingRemainingAccount)?;
        let raw_output_balance = extract_token_amount(titan_pda_output_token_account)?;
        let traded_output_amount =
            raw_output_balance.saturating_sub(reserved_balances[output_ata_index]);
        let output_token_program = get_token_program_for_mint(
            output_mint,
            &ctx.accounts.token_program,
            &ctx.accounts.token_2022_program,
        );

        let user_output = traded_output_amount;

        if !account_exists(&ctx.accounts.output_token_account) {
            create_user_ata(
                &ctx.accounts.payer.to_account_info(),
                &ctx.accounts.user.to_account_info(),
                output_mint,
                &ctx.accounts.output_token_account,
                output_token_program,
                &ctx.accounts.system_program.to_account_info(),
            )?;
        }

        transfer_tokens(
            output_token_program,
            titan_pda_output_token_account,
            &ctx.accounts.output_token_account,
            &ctx.accounts.titan_pda.to_account_info(),
            Some(output_mint),
            user_output,
            output_mint_decimals,
            Some(signer_seeds),
        )?;

        for ata_index in 0..mints_count {
            if created_atas & (1 << ata_index) == 0 {
                continue;
            }

            let ata = ctx
                .remaining_accounts
                .get(ata_index)
                .ok_or(TemplateError::MissingRemainingAccount)?;
            if extract_token_amount(ata).unwrap_or(0) > 0 {
                continue;
            }

            let mint = ctx
                .remaining_accounts
                .get(mints_count + ata_index)
                .ok_or(TemplateError::MissingRemainingAccount)?;
            let token_program = get_token_program_for_mint(
                mint,
                &ctx.accounts.token_program,
                &ctx.accounts.token_2022_program,
            );

            close_titan_pda_ata(
                ata,
                &ctx.accounts.payer.to_account_info(),
                &ctx.accounts.titan_pda.to_account_info(),
                token_program,
                signer_seeds,
            )?;
        }

        let mut balance_idx = 0usize;
        for (i, account) in ctx.remaining_accounts.iter().enumerate() {
            if titan_pda_ata_bitmap.is_set(i) {
                let final_balance = extract_token_amount(account).unwrap_or(0);
                require!(
                    final_balance >= titan_pda_ata_balances[balance_idx],
                    TemplateError::ReservedBalanceViolation
                );
                balance_idx += 1;
            }
        }

        Ok(())
    }
}

fn perform_cpi_swap<'info>(
    amount: u64,
    swap: &SwapSpecInputV2,
    account_metas: &[AccountMeta],
    account_infos: &'info [AccountInfo<'info>],
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let instructions = match swap.venue {
        Venue::RaydiumAmm => raydium_amm::swap_base_in_v2(amount, account_metas)?,
        // FILL_IN: add your venue dispatch arm here.
        Venue::TemplateVenue { zero_for_one } => {
            // Pass every CPI-specific field from the `Venue` variant into your adapter.
            let _ = (zero_for_one, amount, account_metas);
            todo!("replace TemplateVenue dispatch with your venue CPI module")
        }
    };

    for instruction in instructions.iter() {
        invoke_signed(instruction, account_infos, signer_seeds)?;
    }

    Ok(())
}

fn read_token_ledger<'info>(
    token_ledger: &Option<AccountInfo<'info>>,
    input_token_account: &AccountInfo<'info>,
) -> Result<Option<u64>> {
    let Some(ledger_account) = token_ledger else {
        return Ok(None);
    };

    let data = ledger_account.try_borrow_data()?;
    if data.len() < 48 {
        return err!(TemplateError::InvalidAccountData);
    }

    let token_account_slice = data.get(8..40).ok_or(TemplateError::InvalidAccountData)?;
    let token_account = Pubkey::new_from_array(
        token_account_slice
            .try_into()
            .map_err(|_| TemplateError::InvalidAccountData)?,
    );
    require!(
        token_account == input_token_account.key(),
        TemplateError::InvalidAccountData
    );

    let amount_slice = data.get(40..48).ok_or(TemplateError::InvalidAccountData)?;
    let amount = u64::from_le_bytes(
        amount_slice
            .try_into()
            .map_err(|_| TemplateError::InvalidAccountData)?,
    );

    Ok(Some(amount))
}

fn get_token_program_for_mint<'a, 'info>(
    mint: &AccountInfo<'info>,
    token_program: &'a AccountInfo<'info>,
    token_2022_program: &'a AccountInfo<'info>,
) -> &'a AccountInfo<'info> {
    if mint.owner.eq(&TOKEN_PROGRAM_2022_ID) {
        token_2022_program
    } else {
        token_program
    }
}

#[allow(clippy::too_many_arguments)]
fn transfer_tokens<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    mint: Option<&AccountInfo<'info>>,
    amount: u64,
    decimals: u8,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }

    if token_program.key().eq(&TOKEN_PROGRAM_2022_ID) {
        let mint = mint.ok_or(TemplateError::MissingRemainingAccount)?;
        let accounts = TransferChecked {
            from: from.to_account_info(),
            mint: mint.to_account_info(),
            to: to.to_account_info(),
            authority: authority.to_account_info(),
        };

        match signer_seeds {
            Some(seeds) => transfer_checked(
                CpiContext::new_with_signer(token_program.to_account_info(), accounts, seeds),
                amount,
                decimals,
            ),
            None => transfer_checked(
                CpiContext::new(token_program.to_account_info(), accounts),
                amount,
                decimals,
            ),
        }
    } else {
        let accounts = Transfer {
            from: from.to_account_info(),
            to: to.to_account_info(),
            authority: authority.to_account_info(),
        };

        match signer_seeds {
            Some(seeds) => transfer(
                CpiContext::new_with_signer(token_program.to_account_info(), accounts, seeds),
                amount,
            ),
            None => transfer(
                CpiContext::new(token_program.to_account_info(), accounts),
                amount,
            ),
        }
    }
}

fn create_titan_pda_ata<'info>(
    payer: &AccountInfo<'info>,
    titan_pda: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    ata: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
) -> Result<()> {
    let create_ata_ix = create_associated_token_account_idempotent(
        payer.key,
        titan_pda.key,
        mint.key,
        token_program.key,
    );

    invoke(
        &create_ata_ix,
        &[
            payer.to_account_info(),
            ata.to_account_info(),
            titan_pda.to_account_info(),
            mint.to_account_info(),
            system_program.to_account_info(),
            token_program.to_account_info(),
        ],
    )
    .map_err(Into::into)
}

fn create_user_ata<'info>(
    payer: &AccountInfo<'info>,
    owner: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    ata: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
) -> Result<()> {
    let create_ata_ix = create_associated_token_account_idempotent(
        payer.key,
        owner.key,
        mint.key,
        token_program.key,
    );

    invoke(
        &create_ata_ix,
        &[
            payer.to_account_info(),
            ata.to_account_info(),
            owner.to_account_info(),
            mint.to_account_info(),
            system_program.to_account_info(),
            token_program.to_account_info(),
        ],
    )
    .map_err(Into::into)
}

fn close_titan_pda_ata<'info>(
    ata: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
    titan_pda: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    close_account(CpiContext::new_with_signer(
        token_program.to_account_info(),
        CloseAccount {
            account: ata.to_account_info(),
            destination: destination.to_account_info(),
            authority: titan_pda.to_account_info(),
        },
        signer_seeds,
    ))
}

fn close_token_account<'info>(
    token_account: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    let accounts = CloseAccount {
        account: token_account.to_account_info(),
        destination: destination.to_account_info(),
        authority: authority.to_account_info(),
    };

    match signer_seeds {
        Some(seeds) => close_account(CpiContext::new_with_signer(
            token_program.to_account_info(),
            accounts,
            seeds,
        )),
        None => close_account(CpiContext::new(token_program.to_account_info(), accounts)),
    }
}

fn setup_wsol_input_account<'info>(
    payer: &AccountInfo<'info>,
    owner: &AccountInfo<'info>,
    wsol_ata: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    amount: u64,
) -> Result<bool> {
    if account_exists(wsol_ata) {
        let current_balance = extract_token_amount(wsol_ata)?;
        if current_balance < amount {
            let deficit = amount - current_balance;
            transfer_sol_and_sync(owner, wsol_ata, token_program, deficit)?;
        }

        return Ok(false);
    }

    let create_ata_ix = create_associated_token_account_idempotent(
        payer.key,
        owner.key,
        mint.key,
        token_program.key,
    );
    invoke(
        &create_ata_ix,
        &[
            payer.to_account_info(),
            wsol_ata.to_account_info(),
            owner.to_account_info(),
            mint.to_account_info(),
            system_program.to_account_info(),
            token_program.to_account_info(),
        ],
    )?;

    transfer_sol_and_sync(owner, wsol_ata, token_program, amount)?;
    Ok(true)
}

fn transfer_sol_and_sync<'info>(
    from: &AccountInfo<'info>,
    wsol_ata: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }

    let transfer_ix =
        anchor_lang::solana_program::system_instruction::transfer(from.key, wsol_ata.key, amount);
    invoke(
        &transfer_ix,
        &[from.to_account_info(), wsol_ata.to_account_info()],
    )?;

    let sync_ix = spl_token::instruction::sync_native(token_program.key, wsol_ata.key)?;
    invoke(&sync_ix, &[wsol_ata.to_account_info()])?;
    Ok(())
}

fn account_exists(account: &AccountInfo) -> bool {
    account.lamports() > 0 && account.data_len() > 0
}

fn is_native_mint(mint: &AccountInfo) -> bool {
    mint.key().eq(&spl_token::native_mint::ID)
}

fn extract_token_amount(account: &AccountInfo) -> Result<u64> {
    let data = account.try_borrow_data()?;
    let slice = data.get(64..72).ok_or(TemplateError::InvalidAccountData)?;
    Ok(u64::from_le_bytes(
        slice
            .try_into()
            .map_err(|_| TemplateError::InvalidAccountData)?,
    ))
}

fn extract_token_owner(account: &AccountInfo) -> Result<Pubkey> {
    let data = account.try_borrow_data()?;
    let slice = data.get(32..64).ok_or(TemplateError::InvalidAccountData)?;
    Ok(Pubkey::new_from_array(
        slice
            .try_into()
            .map_err(|_| TemplateError::InvalidAccountData)?,
    ))
}

fn extract_mint_decimals(mint: &AccountInfo) -> Result<u8> {
    mint.try_borrow_data()?
        .get(44)
        .copied()
        .ok_or(TemplateError::InvalidAccountData.into())
}

fn is_titan_pda_token_account(account: &AccountInfo, expected_authority: &Pubkey) -> bool {
    if account.owner != &spl_token::ID && account.owner != &TOKEN_PROGRAM_2022_ID {
        return false;
    }
    if account.data_len() < 165 {
        return false;
    }

    let Ok(owner) = extract_token_owner(account) else {
        return false;
    };
    owner == *expected_authority
}

struct TitanPdaBitmap {
    bits: [u64; 4],
}

impl TitanPdaBitmap {
    const fn new() -> Self {
        Self { bits: [0; 4] }
    }

    fn set(&mut self, index: usize) {
        if index < 256 {
            self.bits[index / 64] |= 1u64 << (index % 64);
        }
    }

    fn is_set(&self, index: usize) -> bool {
        index < 256 && self.bits[index / 64] & (1u64 << (index % 64)) != 0
    }
}

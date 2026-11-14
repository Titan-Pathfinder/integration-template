//! TEST ONLY. Appended after the template route in the same local transaction.
//! Models a final-output balance-delta check, NOT Titan's production router.
//! Never deploy or use as a production slippage implementation: the baseline is
//! provided by the test harness, not recorded by a trusted on-chain route entry.
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg,
    program_error::ProgramError, pubkey, pubkey::Pubkey,
};

entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() != 1 || data.len() != 16 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let account = &accounts[0];
    if account.owner != &pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA") {
        return Err(ProgramError::IncorrectProgramId);
    }
    let before = u64::from_le_bytes(data[..8].try_into().unwrap());
    let minimum = u64::from_le_bytes(data[8..].try_into().unwrap());
    let token = account.try_borrow_data()?;
    let amount = token.get(64..72).ok_or(ProgramError::InvalidAccountData)?;
    let after = u64::from_le_bytes(amount.try_into().unwrap());
    let received = after.checked_sub(before).ok_or(ProgramError::Custom(1))?;
    msg!(
        "test-only route output: received={}, minimum={}",
        received,
        minimum
    );
    if received < minimum {
        return Err(ProgramError::Custom(1));
    }
    Ok(())
}

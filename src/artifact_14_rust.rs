use anchor_lang::prelude::*;
use solana_program::ed25519_program;
use solana_program::instruction::Instruction;
use solana_program::sysvar::instructions::{load_instruction_at_checked, ID as IX_SYSVAR_ID};
use crate::state::{UserProfile, ProtocolConfig};
use crate::errors::SolSweepError;
use crate::events::QuestCompletedEvent;

#[derive(Accounts)]
pub struct ClaimQuest<'info> {
    #[account(
        seeds = [b"protocol_config"],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [b"user_profile", user.key().as_ref()],
        bump = profile.bump,
        has_one = owner @ SolSweepError::Unauthorized
    )]
    pub profile: Account<'info, UserProfile>,

    pub user: Signer<'info>,
    /// CHECK: Validated against profile.owner
    pub owner: UncheckedAccount<'info>,

    /// CHECK: Solana Instructions sysvar account for verifying precompile signature
    #[account(address = IX_SYSVAR_ID)]
    pub instructions_sysvar: AccountInfo<'info>,
}

pub fn handler(
    ctx: Context<ClaimQuest>,
    quest_id: u16,
    xp_amount: u64,
    nonce: u64,
    expiry_timestamp: i64,
) -> Result<()> {
    let profile = &mut ctx.accounts.profile;
    let config = &ctx.accounts.config;

    let current_time = Clock::get()?.unix_timestamp;
    require!(current_time <= expiry_timestamp, SolSweepError::QuestExpired);
    require!(nonce > profile.last_nonce, SolSweepError::InvalidNonce);

    // Verify Ed25519 Precompile Signature directly preceding this instruction
    verify_oracle_ed25519_signature(
        &ctx.accounts.instructions_sysvar,
        &config.oracle_authority,
        &profile.owner,
        quest_id,
        xp_amount,
        nonce,
        expiry_timestamp,
    )?;

    // Commit state updates
    profile.xp_points = profile.xp_points.checked_add(xp_amount).ok_or(SolSweepError::MathOverflow)?;
    profile.level = UserProfile::calculate_level(profile.xp_points);
    profile.last_nonce = nonce;

    emit!(QuestCompletedEvent {
        user: profile.owner,
        quest_id,
        xp_earned: xp_amount,
        total_xp: profile.xp_points,
        new_level: profile.level,
        nonce,
        timestamp: current_time,
    });

    Ok(())
}

fn verify_oracle_ed25519_signature(
    ix_sysvar: &AccountInfo,
    expected_oracle: &Pubkey,
    user: &Pubkey,
    quest_id: u16,
    xp_amount: u64,
    nonce: u64,
    expiry: i64,
) -> Result<()> {
    // Expected payload: [user(32) | quest_id(2) | xp_amount(8) | nonce(8) | expiry(8)] = 58 bytes
    let mut message = Vec::with_capacity(58);
    message.extend_from_slice(user.as_ref());
    message.extend_from_slice(&quest_id.to_le_bytes());
    message.extend_from_slice(&xp_amount.to_le_bytes());
    message.extend_from_slice(&nonce.to_le_bytes());
    message.extend_from_slice(&expiry.to_le_bytes());

    // Inspect instruction index 0 in transaction (which must be Ed25519 program)
    let current_ix_index = solana_program::sysvar::instructions::load_current_index_checked(ix_sysvar)?;
    require!(current_ix_index > 0, SolSweepError::InvalidOracleSignature);

    let prior_ix: Instruction = load_instruction_at_checked((current_ix_index - 1) as usize, ix_sysvar)?;
    require!(prior_ix.program_id == ed25519_program::ID, SolSweepError::InvalidOracleSignature);

    // Parse standard Ed25519 instruction data offsets
    // Offset structure: [num_signatures: u8, padding: u8, signature_offset: u16, ...]
    let data = &prior_ix.data;
    require!(data.len() >= 112, SolSweepError::InvalidOracleSignature);

    let public_key_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let message_data_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let message_data_size = u16::from_le_bytes([data[12], data[13]]) as usize;

    require!(message_data_size == message.len(), SolSweepError::InvalidOracleSignature);

    let extracted_pubkey = &data[public_key_offset..public_key_offset + 32];
    require!(extracted_pubkey == expected_oracle.as_ref(), SolSweepError::InvalidOracleSignature);

    let extracted_message = &data[message_data_offset..message_data_offset + message_data_size];
    require!(extracted_message == message.as_slice(), SolSweepError::InvalidOracleSignature);

    Ok(())
}
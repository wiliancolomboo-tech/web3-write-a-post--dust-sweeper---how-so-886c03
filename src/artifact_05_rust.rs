use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::sysvar::instructions as ix_sysvar;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

declare_id!("DustXP1111111111111111111111111111111111111");

pub const CONFIG_SEED: &[u8] = b"config";
pub const USER_PROFILE_SEED: &[u8] = b"user_profile";
pub const NONCE_RECORD_SEED: &[u8] = b"nonce_record";

#[program]
pub mod dust_sweeper_xp {
    use super::*;

    /// Initialize the global configuration and protocol treasury
    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        attestation_authority: Pubkey,
        fee_basis_points: u16, // e.g. 500 = 5% of recovered rent
    ) -> Result<()> {
        require!(fee_basis_points <= 2000, SweeperError::FeeTooHigh); // Max 20%
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.treasury = ctx.accounts.treasury.key();
        config.attestation_authority = attestation_authority;
        config.fee_basis_points = fee_basis_points;
        config.total_rent_reclaimed = 0;
        config.total_accounts_closed = 0;
        config.total_xp_awarded = 0;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    /// Initialize a user's Soulbound Profile PDA
    pub fn initialize_user_profile(ctx: Context<InitializeUserProfile>) -> Result<()> {
        let profile = &mut ctx.accounts.user_profile;
        profile.owner = ctx.accounts.user.key();
        profile.experience_points = 0;
        profile.accounts_closed = 0;
        profile.rent_reclaimed_lamports = 0;
        profile.quests_completed = 0;
        profile.bump = ctx.bumps.user_profile;
        Ok(())
    }

    /// Closes an empty SPL Token Account, returning lamports to the owner and protocol fee to the treasury
    pub fn close_empty_token_account(ctx: Context<CloseEmptyAccount>) -> Result<()> {
        // Enforce account is strictly empty
        let token_account_data = &ctx.accounts.token_account;
        require!(token_account_data.amount == 0, SweeperError::TokenAccountNotEmpty);
        require!(
            token_account_data.owner == ctx.accounts.user.key(),
            SweeperError::UnauthorizedAccountOwner
        );

        let rent_lamports = ctx.accounts.token_account.to_account_info().lamports();
        require!(rent_lamports > 0, SweeperError::ZeroBalanceAccount);

        // Compute protocol fee share
        let fee_lamports = (rent_lamports as u128)
            .checked_mul(ctx.accounts.config.fee_basis_points as u128)
            .ok_or(SweeperError::MathOverflow)?
            .checked_div(10000)
            .ok_or(SweeperError::MathOverflow)? as u64;

        let user_refund = rent_lamports
            .checked_sub(fee_lamports)
            .ok_or(SweeperError::MathOverflow)?;

        // Close SPL Token Account using CPI to token program
        let cpi_accounts = CloseAccount {
            account: ctx.accounts.token_account.to_account_info(),
            destination: ctx.accounts.user.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::close_account(cpi_ctx)?;

        // If a fee is configured, transfer fee from user to treasury
        if fee_lamports > 0 {
            anchor_lang::solana_program::program::invoke(
                &anchor_lang::solana_program::system_instruction::transfer(
                    &ctx.accounts.user.key(),
                    &ctx.accounts.treasury.key(),
                    fee_lamports,
                ),
                &[
                    ctx.accounts.user.to_account_info(),
                    ctx.accounts.treasury.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Award base on-chain sweep XP (50 XP per closed account)
        const BASE_SWEEP_XP: u64 = 50;
        let profile = &mut ctx.accounts.user_profile;
        profile.accounts_closed = profile.accounts_closed.checked_add(1).ok_or(SweeperError::MathOverflow)?;
        profile.rent_reclaimed_lamports = profile
            .rent_reclaimed_lamports
            .checked_add(user_refund)
            .ok_or(SweeperError::MathOverflow)?;
        profile.experience_points = profile
            .experience_points
            .checked_add(BASE_SWEEP_XP)
            .ok_or(SweeperError::MathOverflow)?;

        // Update global analytics
        let config = &mut ctx.accounts.config;
        config.total_accounts_closed = config.total_accounts_closed.checked_add(1).ok_or(SweeperError::MathOverflow)?;
        config.total_rent_reclaimed = config
            .total_rent_reclaimed
            .checked_add(user_refund)
            .ok_or(SweeperError::MathOverflow)?;
        config.total_xp_awarded = config.total_xp_awarded.checked_add(BASE_SWEEP_XP).ok_or(SweeperError::MathOverflow)?;

        emit!(AccountClosedEvent {
            user: ctx.accounts.user.key(),
            token_account: ctx.accounts.token_account.key(),
            reclaimed_lamports: user_refund,
            fee_lamports,
            xp_awarded: BASE_SWEEP_XP,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Verifies an Ed25519-signed off-chain social quest completion and awards XP
    pub fn claim_social_xp(
        ctx: Context<ClaimSocialXp>,
        quest_id: [u8; 16],
        xp_amount: u64,
        nonce: u64,
        signature: [u8; 64],
    ) -> Result<()> {
        require!(xp_amount > 0 && xp_amount <= 5000, SweeperError::InvalidXpAmount);

        // Verify ed25519 signature by introspecting preceding instruction
        let ix_sysvar_account = &ctx.accounts.instruction_sysvar;
        let current_ix_index = ix_sysvar::load_current_index_checked(ix_sysvar_account)? as usize;
        require!(current_ix_index > 0, SweeperError::MissingEd25519Instruction);

        let prev_ix = ix_sysvar::load_instruction_at_checked(current_ix_index - 1, ix_sysvar_account)?;
        require!(
            prev_ix.program_id == anchor_lang::solana_program::ed25519_program::ID,
            SweeperError::InvalidEd25519Program
        );

        // Serialize the expected payload: [user_pubkey (32) | quest_id (16) | xp_amount (8 LE) | nonce (8 LE)]
        let mut expected_message = Vec::with_capacity(64);
        expected_message.extend_from_slice(ctx.accounts.user.key().as_ref());
        expected_message.extend_from_slice(&quest_id);
        expected_message.extend_from_slice(&xp_amount.to_le_bytes());
        expected_message.extend_from_slice(&nonce.to_le_bytes());

        // Validate that Ed25519 instruction contains our authority and serialized message
        verify_ed25519_instruction_payload(
            &prev_ix,
            &ctx.accounts.config.attestation_authority,
            &expected_message,
            &signature,
        )?;

        // Mark nonce as consumed
        let nonce_record = &mut ctx.accounts.nonce_record;
        nonce_record.user = ctx.accounts.user.key();
        nonce_record.nonce = nonce;
        nonce_record.quest_id = quest_id;
        nonce_record.bump = ctx.bumps.nonce_record;

        // Credit XP to User Profile
        let profile = &mut ctx.accounts.user_profile;
        profile.experience_points = profile
            .experience_points
            .checked_add(xp_amount)
            .ok_or(SweeperError::MathOverflow)?;
        profile.quests_completed = profile
            .quests_completed
            .checked_add(1)
            .ok_or(SweeperError::MathOverflow)?;

        // Update Config Registry stats
        let config = &mut ctx.accounts.config;
        config.total_xp_awarded = config
            .total_xp_awarded
            .checked_add(xp_amount)
            .ok_or(SweeperError::MathOverflow)?;

        emit!(SocialXpClaimedEvent {
            user: ctx.accounts.user.key(),
            quest_id,
            xp_awarded: xp_amount,
            new_total_xp: profile.experience_points,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}

/// Helper function to introspect Ed25519 precompile instruction buffer
fn verify_ed25519_instruction_payload(
    ix: &Instruction,
    expected_pubkey: &Pubkey,
    expected_message: &[u8],
    expected_signature: &[u8; 64],
) -> Result<()> {
    let data = &ix.data;
    // Ed25519 precompile header is at least 14 bytes + signature(64) + pubkey(32) + message
    require!(data.len() >= 112, SweeperError::MalformedEd25519Instruction);

    // Verify 1 signature was verified
    let num_signatures = data[0];
    require!(num_signatures == 1, SweeperError::MalformedEd25519Instruction);

    // Parse offsets according to Solana Ed25519 precompile specification
    let signature_offset = u16::from_le_bytes([data[2], data[3]]) as usize;
    let public_key_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let message_data_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let message_data_size = u16::from_le_bytes([data[12], data[13]]) as usize;

    require!(signature_offset + 64 <= data.len(), SweeperError::MalformedEd25519Instruction);
    require!(public_key_offset + 32 <= data.len(), SweeperError::MalformedEd25519Instruction);
    require!(message_data_offset + message_data_size <= data.len(), SweeperError::MalformedEd25519Instruction);

    let extracted_signature = &data[signature_offset..signature_offset + 64];
    let extracted_pubkey = &data[public_key_offset..public_key_offset + 32];
    let extracted_message = &data[message_data_offset..message_data_offset + message_data_size];

    require!(extracted_signature == expected_signature.as_slice(), SweeperError::SignatureMismatch);
    require!(extracted_pubkey == expected_pubkey.as_ref(), SweeperError::UnauthorizedSigner);
    require!(extracted_message == expected_message, SweeperError::MessageMismatch);

    Ok(())
}

// ==========================================
// Account Contexts
// ==========================================

#[derive(Accounts)]
pub ছবিতে
pub struct InitializeConfig<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, Config>,
    /// CHECK: Treasury account receiving rent splits
    pub treasury: AccountInfo<'info>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeUserProfile<'info> {
    #[account(
        init,
        payer = user,
        space = 8 + UserProfile::INIT_SPACE,
        seeds = [USER_PROFILE_SEED, user.key().as_ref()],
        bump
    )]
    pub user_profile: Account<'info, UserProfile>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CloseEmptyAccount<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [USER_PROFILE_SEED, user.key().as_ref()],
        bump = user_profile.bump
    )]
    pub user_profile: Account<'info, UserProfile>,
    #[account(mut)]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: Verified against config.treasury
    #[account(mut, address = config.treasury @ SweeperError::InvalidTreasury)]
    pub treasury: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(quest_id: [u8; 16], xp_amount: u64, nonce: u64)]
pub struct ClaimSocialXp<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [USER_PROFILE_SEED, user.key().as_ref()],
        bump = user_profile.bump
    )]
    pub user_profile: Account<'info, UserProfile>,
    #[account(
        init,
        payer = user,
        space = 8 + NonceRecord::INIT_SPACE,
        seeds = [NONCE_RECORD_SEED, user.key().as_ref(), &nonce.to_le_bytes()],
        bump
    )]
    pub nonce_record: Account<'info, NonceRecord>,
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: Instructions sysvar account for precompile introspection
    #[account(address = ix_sysvar::ID @ SweeperError::InvalidInstructionSysvar)]
    pub instruction_sysvar: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

// ==========================================
// Data Structures & State
// ==========================================

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub attestation_authority: Pubkey,
    pub fee_basis_points: u16,
    pub total_rent_reclaimed: u64,
    pub total_accounts_closed: u64,
    pub total_xp_awarded: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct UserProfile {
    pub owner: Pubkey,
    pub experience_points: u64,
    pub accounts_closed: u64,
    pub rent_reclaimed_lamports: u64,
    pub quests_completed: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct NonceRecord {
    pub user: Pubkey,
    pub nonce: u64,
    pub quest_id: [u8; 16],
    pub bump: u8,
}

// ==========================================
// Events & Errors
// ==========================================

#[event]
pub struct AccountClosedEvent {
    pub user: Pubkey,
    pub token_account: Pubkey,
    pub reclaimed_lamports: u64,
    pub fee_lamports: u64,
    pub xp_awarded: u64,
    pub timestamp: i64,
}

#[event]
pub struct SocialXpClaimedEvent {
    pub user: Pubkey,
    pub quest_id: [u8; 16],
    pub xp_awarded: u64,
    pub new_total_xp: u64,
    pub timestamp: i64,
}

#[error_code]
pub enum SweeperError {
    #[msg("Protocol fee basis points cannot exceed 2000 (20%)")]
    FeeTooHigh,
    #[msg("Cannot close a token account with non-zero balance. Sweep dust first.")]
    TokenAccountNotEmpty,
    #[msg("Signer is not the authorized owner of this token account")]
    UnauthorizedAccountOwner,
    #[msg("Token account contains zero lamports")]
    ZeroBalanceAccount,
    #[msg("Invalid treasury account address")]
    InvalidTreasury,
    #[msg("Arithmetic overflow in fee or XP calculation")]
    MathOverflow,
    #[msg("XP amount must be between 1 and 5000")]
    InvalidXpAmount,
    #[msg("Instruction sysvar account is invalid")]
    InvalidInstructionSysvar,
    #[msg("Ed25519 signature instruction is missing from transaction")]
    MissingEd25519Instruction,
    #[msg("Instruction preceding claim must be the Ed25519 native precompile program")]
    InvalidEd25519Program,
    #[msg("Ed25519 instruction payload is malformed or corrupted")]
    MalformedEd25519Instruction,
    #[msg("Provided signature does not match Ed25519 precompile verification")]
    SignatureMismatch,
    #[msg("Attestation signature was not signed by the protocol authority")]
    UnauthorizedSigner,
    #[msg("Signed message content does not match execution parameters")]
    MessageMismatch,
}
use anchor_lang::prelude::*;
use anchor_lang::solana_program::ed25519_program;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::sysvar::instructions::{
    load_current_index_checked, load_instruction_at_checked, ID as IX_SYSVAR_ID,
};
use anchor_spl::token::{self, Burn, CloseAccount, Mint, Token, TokenAccount};

declare_id!("SweepR8mQjH27Nq4gP5WbYyvVd1X8G4pLwK3qH7vZ5m");

pub const PROFILE_SEED: &[u8] = b"user_profile";
pub const CONFIG_SEED: &[u8] = b"global_config";
pub const SWEEP_XP_REWARD_PER_ACCOUNT: u64 = 50;

#[program]
pub mod dust_sweeper_xp {
    use super::*;

    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        oracle_signer: Pubkey,
        fee_basis_points: u16,
    ) -> Result<()> {
        require!(fee_basis_points <= 1000, CustomError::FeeTooHigh); // Max 10%
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.oracle_signer = oracle_signer;
        config.fee_basis_points = fee_basis_points;
        config.total_sol_reclaimed = 0;
        config.total_accounts_closed = 0;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    pub fn initialize_user_profile(ctx: Context<InitializeUserProfile>) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        profile.owner = ctx.accounts.user.key();
        profile.total_xp = 0;
        profile.accounts_swept = 0;
        profile.lamports_reclaimed = 0;
        profile.completed_quests_bitmask = 0;
        profile.bump = ctx.bumps.profile;
        Ok(())
    }

    pub fn sweep_dust_and_close_account(ctx: Context<SweepDustAndClose>) -> Result<()> {
        let token_amount = ctx.accounts.token_account.amount;
        let rent_lamports = ctx.accounts.token_account.to_account_info().lamports();

        // If residual balance exists, burn it (dust destruction)
        if token_amount > 0 {
            let burn_ctx = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.token_mint.to_account_info(),
                    from: ctx.accounts.token_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            );
            token::burn(burn_ctx, token_amount)?;
        }

        // Close SPL Token account to refund rent directly to user
        let close_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.token_account.to_account_info(),
                destination: ctx.accounts.user.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::close_account(close_ctx)?;

        // Update User Profile metrics
        let profile = &mut ctx.accounts.profile;
        profile.accounts_swept = profile.accounts_swept.checked_add(1).ok_or(CustomError::MathOverflow)?;
        profile.lamports_reclaimed = profile
            .lamports_reclaimed
            .checked_add(rent_lamports)
            .ok_or(CustomError::MathOverflow)?;
        profile.total_xp = profile
            .total_xp
            .checked_add(SWEEP_XP_REWARD_PER_ACCOUNT)
            .ok_or(CustomError::MathOverflow)?;

        // Update Protocol Config metrics
        let config = &mut ctx.accounts.config;
        config.total_accounts_closed = config
            .total_accounts_closed
            .checked_add(1)
            .ok_or(CustomError::MathOverflow)?;
        config.total_sol_reclaimed = config
            .total_sol_reclaimed
            .checked_add(rent_lamports)
            .ok_or(CustomError::MathOverflow)?;

        emit!(AccountSweptEvent {
            user: ctx.accounts.user.key(),
            token_account: ctx.accounts.token_account.key(),
            dust_burned: token_amount,
            rent_reclaimed: rent_lamports,
            xp_awarded: SWEEP_XP_REWARD_PER_ACCOUNT,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    pub fn verify_and_claim_social_quest(
        ctx: Context<VerifySocialQuest>,
        quest_id: u8,
        xp_reward: u64,
        nonce: u64,
        sig_data: [u8; 64],
    ) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        let config = &ctx.accounts.config;

        // 1. Bitmask replay prevention check
        require!(quest_id < 64, CustomError::InvalidQuestId);
        let quest_mask: u64 = 1 << quest_id;
        require!(
            (profile.completed_quests_bitmask & quest_mask) == 0,
            CustomError::QuestAlreadyCompleted
        );

        // 2. Validate Ed25519 signature through Sysvar Instructions Pre-Instruction Verification
        let ix_sysvar = &ctx.accounts.ix_sysvar;
        let current_ix_index = load_current_index_checked(ix_sysvar)?;
        require!(current_ix_index > 0, CustomError::MissingSignatureVerification);

        let ed25519_ix: Instruction = load_instruction_at_checked(
            (current_ix_index - 1) as usize,
            ix_sysvar,
        )?;

        // Verify ed25519 program id
        require!(
            ed25519_ix.program_id == ed25519_program::ID,
            CustomError::InvalidEd25519Program
        );

        // Expected message payload constructed off-chain by Oracle:
        // [user_pubkey (32 bytes) | quest_id (1 byte) | xp_reward (8 bytes LE) | nonce (8 bytes LE)]
        let mut expected_message = Vec::with_capacity(49);
        expected_message.extend_from_slice(ctx.accounts.user.key().as_ref());
        expected_message.push(quest_id);
        expected_message.extend_from_slice(&xp_reward.to_le_bytes());
        expected_message.extend_from_slice(&nonce.to_le_bytes());

        // Validate that Ed25519 ix data payload matches our expected message and trusted oracle pubkey
        validate_ed25519_ix_data(
            &ed25519_ix.data,
            config.oracle_signer.as_ref(),
            &expected_message,
            &sig_data,
        )?;

        // 3. Mark quest as completed and credit XP
        profile.completed_quests_bitmask |= quest_mask;
        profile.total_xp = profile
            .total_xp
            .checked_add(xp_reward)
            .ok_or(CustomError::MathOverflow)?;

        emit!(SocialQuestCompletedEvent {
            user: ctx.accounts.user.key(),
            quest_id,
            xp_reward,
            total_xp: profile.total_xp,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}

/// Helper function to introspect Solana native Ed25519 program instruction format
fn validate_ed25519_ix_data(
    data: &[u8],
    oracle_pubkey: &[u8],
    message: &[u8],
    sig: &[u8; 64],
) -> Result<()> {
    // Solana Ed25519 Instruction format has a 14-byte header:
    // num_signatures: u8
    // padding: u8
    // signature_offset: u16
    // signature_instruction_index: u16
    // public_key_offset: u16
    // public_key_instruction_index: u16
    // message_data_offset: u16
    // message_data_size: u16
    // message_instruction_index: u16
    require!(data.len() >= 14 + 64 + 32 + message.len(), CustomError::MalformedEd25519Instruction);

    let pubkey_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let signature_offset = u16::from_le_bytes([data[2], data[3]]) as usize;
    let msg_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let msg_size = u16::from_le_bytes([data[12], data[13]]) as usize;

    require!(msg_size == message.len(), CustomError::MessageLengthMismatch);
    require!(&data[pubkey_offset..pubkey_offset + 32] == oracle_pubkey, CustomError::UnauthorizedOracle);
    require!(&data[signature_offset..signature_offset + 64] == sig, CustomError::InvalidSignature);
    require!(&data[msg_offset..msg_offset + msg_size] == message, CustomError::MessageContentMismatch);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + ProtocolConfig::LEN,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, ProtocolConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeUserProfile<'info> {
    #[account(
        init,
        payer = user,
        space = 8 + UserProfile::LEN,
        seeds = [PROFILE_SEED, user.key().as_ref()],
        bump
    )]
    pub profile: Account<'info, UserProfile>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SweepDustAndClose<'info> {
    #[account(
        mut,
        seeds = [PROFILE_SEED, user.key().as_ref()],
        bump = profile.bump,
        has_one = owner @ CustomError::UnauthorizedOwner
    )]
    pub profile: Account<'info, UserProfile>,
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,
    #[account(
        mut,
        constraint = token_account.owner == user.key() @ CustomError::InvalidTokenAccountOwner,
        constraint = token_account.mint == token_mint.key() @ CustomError::InvalidMint
    )]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_mint: Account<'info, Mint>,
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: Safe validation
    pub owner: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct VerifySocialQuest<'info> {
    #[account(
        mut,
        seeds = [PROFILE_SEED, user.key().as_ref()],
        bump = profile.bump
    )]
    pub profile: Account<'info, UserProfile>,
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: Instructions sysvar account for pre-instruction introspection
    #[account(address = IX_SYSVAR_ID)]
    pub ix_sysvar: AccountInfo<'info>,
}

#[account]
pub struct ProtocolConfig {
    pub admin: Pubkey,               // 32
    pub oracle_signer: Pubkey,       // 32
    pub fee_basis_points: u16,       // 2
    pub total_sol_reclaimed: u64,    // 8
    pub total_accounts_closed: u64,  // 8
    pub bump: u8,                    // 1
}

impl ProtocolConfig {
    pub const LEN: usize = 32 + 32 + 2 + 8 + 8 + 1;
}

#[account]
pub struct UserProfile {
    pub owner: Pubkey,                   // 32
    pub total_xp: u64,                   // 8
    pub accounts_swept: u64,             // 8
    pub lamports_reclaimed: u64,         // 8
    pub completed_quests_bitmask: u64,   // 8
    pub bump: u8,                        // 1
}

impl UserProfile {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 8 + 1;
}

#[event]
pub struct AccountSweptEvent {
    pub user: Pubkey,
    pub token_account: Pubkey,
    pub dust_burned: u64,
    pub rent_reclaimed: u64,
    pub xp_awarded: u64,
    pub timestamp: i64,
}

#[event]
pub struct SocialQuestCompletedEvent {
    pub user: Pubkey,
    pub quest_id: u8,
    pub xp_reward: u64,
    pub total_xp: u64,
    pub timestamp: i64,
}

#[error_code]
pub enum CustomError {
    #[msg("Administrative fee exceeds maximum threshold.")]
    FeeTooHigh,
    #[msg("Arithmetic overflow in computational tracking.")]
    MathOverflow,
    #[msg("Signer does not match profile owner.")]
    UnauthorizedOwner,
    #[msg("Token account does not belong to signer.")]
    InvalidTokenAccountOwner,
    #[msg("Provided token mint does not match account mint.")]
    InvalidMint,
    #[msg("Quest ID is out of range [0-63].")]
    InvalidQuestId,
    #[msg("Social quest has already been claimed by this profile.")]
    QuestAlreadyCompleted,
    #[msg("Missing required Ed25519 signature verification pre-instruction.")]
    MissingSignatureVerification,
    #[msg("Instruction is not a valid Ed25519 program invocation.")]
    InvalidEd25519Program,
    #[msg("Ed25519 instruction data is malformed.")]
    MalformedEd25519Instruction,
    #[msg("Message length does not match expected quest data schema.")]
    MessageLengthMismatch,
    #[msg("Attestation signature was not generated by registered protocol oracle.")]
    UnauthorizedOracle,
    #[msg("Invalid cryptographic signature provided for quest verification.")]
    InvalidSignature,
    #[msg("Message content does not match signed digest.")]
    MessageContentMismatch,
}
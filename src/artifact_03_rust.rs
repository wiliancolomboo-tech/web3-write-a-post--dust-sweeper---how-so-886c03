use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, CloseAccount, Token, TokenAccount};
use solana_program::ed25519_program;
use solana_program::instruction::Instruction;
use solana_program::sysvar::instructions as ix_sysvar;

declare_id!("DustXP1111111111111111111111111111111111111");

pub const CONFIG_SEED: &[u8] = b"global_config";
pub const USER_PROFILE_SEED: &[u8] = b"user_profile";
pub const QUEST_RECORD_SEED: &[u8] = b"quest_record";

// Error Codes
#[error_code]
pub enum DustSweeperError {
    #[msg("Account balance exceeds dust threshold. Swap tokens before sweeping.")]
    AccountBalanceNotZeroOrDust,
    #[msg("Invalid Oracle signature for quest verification.")]
    InvalidOracleSignature,
    #[msg("Quest has already been completed by this user.")]
    QuestAlreadyCompleted,
    #[msg("Quest verification proof has expired.")]
    QuestProofExpired,
    #[msg("Numerical overflow occurred during computation.")]
    MathOverflow,
    #[msg("Unauthorized signer for administrative operations.")]
    UnauthorizedAdmin,
}

#[program]
pub mod dust_sweeper_xp {
    use super::*;

    /// Initializes global protocol state, configuring oracle verification keys.
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        oracle_signer: Pubkey,
        max_dust_threshold: u64,
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.oracle_signer = oracle_signer;
        config.max_dust_threshold = max_dust_threshold;
        config.total_accounts_closed = 0;
        config.total_rent_reclaimed = 0;
        config.total_xp_minted = 0;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    /// Initializes a user profile PDA for tracking XP, rank, and sweep metrics.
    pub fn initialize_user_profile(ctx: Context<InitializeUserProfile>) -> Result<()> {
        let profile = &mut ctx.accounts.user_profile;
        profile.owner = ctx.accounts.user.key();
        profile.experience_points = 0;
        profile.current_tier = 1;
        profile.accounts_swept = 0;
        profile.rent_reclaimed_lamports = 0;
        profile.streak_counter = 0;
        profile.last_sweep_timestamp = Clock::get()?.unix_timestamp;
        profile.bump = ctx.bumps.user_profile;
        Ok(())
    }

    /// Sweeps an SPL Token account: Burns any residual micro-dust (<= threshold),
    /// closes the account, credits reclaimed rent directly to user, and awards XP.
    pub fn sweep_and_close_account(ctx: Context<SweepAndCloseAccount>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let token_account = &ctx.accounts.token_account;
        let current_balance = token_account.amount;

        // Security assertion: balance must be 0 or under configured threshold
        require!(
            current_balance <= config.max_dust_threshold,
            DustSweeperError::AccountBalanceNotZeroOrDust
        );

        // 1. If dust remains, burn it to enable closing the account
        if current_balance > 0 {
            let burn_ctx = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: token_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            );
            token::burn(burn_ctx, current_balance)?;
        }

        // 2. Fetch the initial rent lamports being liberated
        let rent_lamports = token_account.to_account_info().lamports();

        // 3. Close the SPL Token Account and refund rent lamports to user
        let close_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: token_account.to_account_info(),
                destination: ctx.accounts.user.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::close_account(close_ctx)?;

        // 4. Update Protocol Aggregates
        config.total_accounts_closed = config
            .total_accounts_closed
            .checked_add(1)
            .ok_or(DustSweeperError::MathOverflow)?;
        config.total_rent_reclaimed = config
            .total_rent_reclaimed
            .checked_add(rent_lamports)
            .ok_or(DustSweeperError::MathOverflow)?;

        // 5. Update User Profile XP & Streaks
        let profile = &mut ctx.accounts.user_profile;
        let now = Clock::get()?.unix_timestamp;

        // Daily streak calculation (24-48 hours window)
        let delta_time = now.saturating_sub(profile.last_sweep_timestamp);
        if delta_time >= 86400 && delta_time <= 172800 {
            profile.streak_counter = profile.streak_counter.saturating_add(1);
        } else if delta_time > 172800 {
            profile.streak_counter = 1;
        }

        profile.accounts_swept = profile
            .accounts_swept
            .checked_add(1)
            .ok_or(DustSweeperError::MathOverflow)?;
        profile.rent_reclaimed_lamports = profile
            .rent_reclaimed_lamports
            .checked_add(rent_lamports)
            .ok_or(DustSweeperError::MathOverflow)?;
        profile.last_sweep_timestamp = now;

        // Base XP: 50 XP per closed account + (5 XP * streak multiplier)
        let streak_bonus = (profile.streak_counter as u64).saturating_mul(5);
        let earned_xp = (50u64).saturating_add(streak_bonus);

        profile.experience_points = profile
            .experience_points
            .checked_add(earned_xp)
            .ok_or(DustSweeperError::MathOverflow)?;

        // Recompute dynamic tier: Level = 1 + floor(sqrt(XP / 100))
        let new_tier = 1 + ((profile.experience_points / 100) as f64).sqrt() as u8;
        profile.current_tier = new_tier;

        config.total_xp_minted = config
            .total_xp_minted
            .checked_add(earned_xp)
            .ok_or(DustSweeperError::MathOverflow)?;

        emit!(AccountSweptEvent {
            user: ctx.accounts.user.key(),
            token_account: ctx.accounts.token_account.key(),
            rent_reclaimed: rent_lamports,
            xp_earned: earned_xp,
            new_total_xp: profile.experience_points,
            current_tier: profile.current_tier,
            timestamp: now,
        });

        Ok(())
    }

    /// Verifies off-chain social quest completion (X follow/repost, Discord join)
    /// authenticated by the Oracle's cryptographic Ed25519 signature.
    pub fn claim_social_quest_xp(
        ctx: Context<ClaimSocialQuestXP>,
        quest_id: [u8; 16],
        xp_reward: u64,
        expiry_timestamp: i64,
        signature: [u8; 64],
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let now = Clock::get()?.unix_timestamp;

        // Validate timestamp freshness
        require!(now <= expiry_timestamp, DustSweeperError::QuestProofExpired);

        // Construct expected payload message:
        // message = user_pubkey (32) + quest_id (16) + xp_reward (8 LE) + expiry (8 LE)
        let mut message = Vec::with_capacity(64);
        message.extend_from_slice(ctx.accounts.user.key().as_ref());
        message.extend_from_slice(&quest_id);
        message.extend_from_slice(&xp_reward.to_le_bytes());
        message.extend_from_slice(&expiry_timestamp.to_le_bytes());

        // Verify cryptographic Ed25519 instruction within the transaction introspect sysvar
        verify_ed25519_instruction(
            &ctx.accounts.instructions_sysvar.to_account_info(),
            &config.oracle_signer.to_bytes(),
            &message,
            &signature,
        )?;

        // Record quest completion to prevent replay attacks
        let quest_record = &mut ctx.accounts.quest_record;
        quest_record.user = ctx.accounts.user.key();
        quest_record.quest_id = quest_id;
        quest_record.xp_awarded = xp_reward;
        quest_record.completed_at = now;
        quest_record.bump = ctx.bumps.quest_record;

        // Credit XP to User Profile
        let profile = &mut ctx.accounts.user_profile;
        profile.experience_points = profile
            .experience_points
            .checked_add(xp_reward)
            .ok_or(DustSweeperError::MathOverflow)?;

        // Recompute dynamic tier
        let new_tier = 1 + ((profile.experience_points / 100) as f64).sqrt() as u8;
        profile.current_tier = new_tier;

        config.total_xp_minted = config
            .total_xp_minted
            .checked_add(xp_reward)
            .ok_or(DustSweeperError::MathOverflow)?;

        emit!(SocialQuestClaimedEvent {
            user: ctx.accounts.user.key(),
            quest_id,
            xp_reward,
            new_total_xp: profile.experience_points,
            current_tier: profile.current_tier,
            timestamp: now,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Cryptographic Proof Verifier (Sysvar Introspection)
// -----------------------------------------------------------------------------
fn verify_ed25519_instruction(
    instructions_sysvar: &AccountInfo,
    expected_pubkey: &[u8; 32],
    expected_message: &[u8],
    expected_signature: &[u8; 64],
) -> Result<()> {
    let current_ix_index = ix_sysvar::load_current_index_checked(instructions_sysvar)?;

    // Guard: Ed25519 pre-instruction must precede current instruction
    require!(current_ix_index > 0, DustSweeperError::InvalidOracleSignature);

    let ed25519_ix: Instruction = ix_sysvar::load_instruction_at_checked(
        (current_ix_index - 1) as usize,
        instructions_sysvar,
    )?;

    // Ensure target program is ed25519 native program
    require!(
        ed25519_ix.program_id == ed25519_program::ID,
        DustSweeperError::InvalidOracleSignature
    );

    // Ed25519 instruction data serialization layout check
    let data = ed25519_ix.data;
    require!(data.len() >= 16 + 32 + 64 + expected_message.len(), DustSweeperError::InvalidOracleSignature);

    // Extract signature, public key, and payload from offsets
    let pubkey_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let sig_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let msg_offset = u16::from_le_bytes([data[14], data[15]]) as usize;
    let msg_size = u16::from_le_bytes([data[12], data[13]]) as usize;

    let extracted_pubkey = &data[pubkey_offset..pubkey_offset + 32];
    let extracted_sig = &data[sig_offset..sig_offset + 64];
    let extracted_msg = &data[msg_offset..msg_offset + msg_size];

    require!(extracted_pubkey == expected_pubkey, DustSweeperError::InvalidOracleSignature);
    require!(extracted_sig == expected_signature, DustSweeperError::InvalidOracleSignature);
    require!(extracted_msg == expected_message, DustSweeperError::InvalidOracleSignature);

    Ok(())
}

// -----------------------------------------------------------------------------
// Account Context Structs
// -----------------------------------------------------------------------------
#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + GlobalConfig::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, GlobalConfig>,
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
pub struct SweepAndCloseAccount<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, GlobalConfig>,

    #[account(
        mut,
        seeds = [USER_PROFILE_SEED, user.key().as_ref()],
        bump = user_profile.bump,
        has_one = owner @ DustSweeperError::UnauthorizedAdmin
    )]
    pub user_profile: Account<'info, UserProfile>,

    #[account(
        mut,
        constraint = token_account.owner == user.key()
    )]
    pub token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = token_account.mint == mint.key()
    )]
    pub mint: Account<'info, anchor_spl::token::Mint>,

    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: Validated as owner of profile and signer
    pub owner: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(quest_id: [u8; 16])]
pub struct ClaimSocialQuestXP<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, GlobalConfig>,

    #[account(
        mut,
        seeds = [USER_PROFILE_SEED, user.key().as_ref()],
        bump = user_profile.bump,
        has_one = owner @ DustSweeperError::UnauthorizedAdmin
    )]
    pub user_profile: Account<'info, UserProfile>,

    #[account(
        init,
        payer = user,
        space = 8 + QuestRecord::INIT_SPACE,
        seeds = [QUEST_RECORD_SEED, user.key().as_ref(), quest_id.as_ref()],
        bump
    )]
    pub quest_record: Account<'info, QuestRecord>,

    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: Validated as owner of profile and signer
    pub owner: UncheckedAccount<'info>,

    /// CHECK: Solana Instruction Introspection Sysvar
    #[account(address = ix_sysvar::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

// -----------------------------------------------------------------------------
// State PDA Models
// -----------------------------------------------------------------------------
#[account]
#[derive(InitSpace)]
pub struct GlobalConfig {
    pub admin: Pubkey,
    pub oracle_signer: Pubkey,
    pub max_dust_threshold: u64,
    pub total_accounts_closed: u64,
    pub total_rent_reclaimed: u64,
    pub total_xp_minted: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct UserProfile {
    pub owner: Pubkey,
    pub experience_points: u64,
    pub current_tier: u8,
    pub accounts_swept: u64,
    pub rent_reclaimed_lamports: u64,
    pub streak_counter: u32,
    pub last_sweep_timestamp: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct QuestRecord {
    pub user: Pubkey,
    pub quest_id: [u8; 16],
    pub xp_awarded: u64,
    pub completed_at: i64,
    pub bump: u8,
}

// -----------------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------------
#[event]
pub struct AccountSweptEvent {
    pub user: Pubkey,
    pub token_account: Pubkey,
    pub rent_reclaimed: u64,
    pub xp_earned: u64,
    pub new_total_xp: u64,
    pub current_tier: u8,
    pub timestamp: i64,
}

#[event]
pub struct SocialQuestClaimedEvent {
    pub user: Pubkey,
    pub quest_id: [u8; 16],
    pub xp_reward: u64,
    pub new_total_xp: u64,
    pub current_tier: u8,
    pub timestamp: i64,
}
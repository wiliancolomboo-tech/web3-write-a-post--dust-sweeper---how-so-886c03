use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

declare_id!("DustSwp11111111111111111111111111111111111");

pub const CONFIG_SEED: &[u8] = b"config";
pub const USER_PROFILE_SEED: &[u8] = b"user_profile";
pub const MAX_QUEST_ID: u8 = 64;

#[program]
pub mod dust_sweeper_engine {
    use super::*;

    /// Initializes the protocol configuration and designates the oracle/authority for social quests.
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        oracle_authority: Pubkey,
        fee_basis_points: u16,
    ) -> Result<()> {
        require!(fee_basis_points <= 1000, SweeperError::FeeTooHigh); // Max 10%

        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.oracle_authority = oracle_authority;
        config.fee_basis_points = fee_basis_points;
        config.total_rent_reclaimed_lamports = 0;
        config.total_dust_accounts_closed = 0;
        config.bump = ctx.bumps.config;

        emit!(ProtocolInitializedEvent {
            admin: config.admin,
            oracle_authority,
            fee_basis_points,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Initializes an on-chain User Profile and XP Ledger.
    pub fn initialize_user_profile(ctx: Context<InitializeUserProfile>) -> Result<()> {
        let profile = &mut ctx.accounts.user_profile;
        profile.owner = ctx.accounts.owner.key();
        profile.xp = 0;
        profile.tier = 1;
        profile.accounts_closed = 0;
        profile.rent_reclaimed_lamports = 0;
        profile.quests_completed_mask = 0;
        profile.bump = ctx.bumps.user_profile;

        emit!(UserProfileCreatedEvent {
            owner: profile.owner,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Closes a zero-balance or dust token account, returning all rent SOL to the owner
    /// and awarding Dust Sweeper XP to the user profile.
    pub fn sweep_and_close_account(ctx: Context<SweepAndCloseAccount>) -> Result<()> {
        let token_account = &ctx.accounts.token_account;
        
        // Zero balance check: Token must be empty to close safely
        require!(token_account.amount == 0, SweeperError::TokenAccountNotEmpty);
        require!(
            token_account.owner == ctx.accounts.owner.key(),
            SweeperError::UnauthorizedTokenOwner
        );

        let rent_lamports = token_account.to_account_info().lamports();
        require!(rent_lamports > 0, SweeperError::ZeroRentAvailable);

        // Perform CPI to SPL Token program to close account and return lamports to owner
        let cpi_accounts = CloseAccount {
            account: token_account.to_account_info(),
            destination: ctx.accounts.owner.to_account_info(),
            authority: ctx.accounts.owner.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::close_account(cpi_ctx)?;

        // Update User Profile Stats & XP
        let profile = &mut ctx.accounts.user_profile;
        let config = &mut ctx.accounts.config;

        let xp_earned: u64 = 25; // 25 Base XP per swept account
        profile.xp = profile.xp.saturating_add(xp_earned);
        profile.accounts_closed = profile.accounts_closed.saturating_add(1);
        profile.rent_reclaimed_lamports = profile
            .rent_reclaimed_lamports
            .saturating_add(rent_lamports);
        
        // Update user tier dynamically based on XP thresholds
        profile.tier = calculate_tier(profile.xp);

        // Protocol level aggregation
        config.total_dust_accounts_closed = config.total_dust_accounts_closed.saturating_add(1);
        config.total_rent_reclaimed_lamports = config
            .total_rent_reclaimed_lamports
            .saturating_add(rent_lamports);

        emit!(AccountSweptEvent {
            owner: profile.owner,
            token_account: token_account.key(),
            rent_reclaimed: rent_lamports,
            xp_awarded: xp_earned,
            new_total_xp: profile.xp,
            new_tier: profile.tier,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Verifies off-chain social quest completion validated by oracle signature.
    /// Bitmask ensures each quest_id (0-63) can only be redeemed once.
    pub fn verify_and_credit_quest(
        ctx: Context<VerifyAndCreditQuest>,
        quest_id: u8,
        xp_value: u64,
        signature: [u8; 64],
    ) -> Result<()> {
        require!(quest_id < MAX_QUEST_ID, SweeperError::InvalidQuestId);
        require!(xp_value > 0 && xp_value <= 5000, SweeperError::InvalidXpValue);

        let profile = &mut ctx.accounts.user_profile;
        let config = &ctx.accounts.config;

        // Anti-replay: Check bitmask
        let quest_mask: u64 = 1u64 << quest_id;
        require!(
            (profile.quests_completed_mask & quest_mask) == 0,
            SweeperError::QuestAlreadyCompleted
        );

        // Cryptographic verification payload: [wallet_pubkey (32), quest_id (1), xp_value (8 LE)]
        let mut message = Vec::with_capacity(41);
        message.extend_from_slice(profile.owner.as_ref());
        message.push(quest_id);
        message.extend_from_slice(&xp_value.to_le_bytes());

        // Verify ed25519 signature from configured oracle_authority
        // Note: For native Solana instructions, this can be coupled with Ed25519Program pre-instructions.
        // Here we validate the attestation authority matches the configuration.
        require!(
            ctx.accounts.oracle_signer.key() == config.oracle_authority,
            SweeperError::InvalidOracleAuthority
        );

        // Mark quest as completed
        profile.quests_completed_mask |= quest_mask;
        profile.xp = profile.xp.saturating_add(xp_value);
        profile.tier = calculate_tier(profile.xp);

        emit!(SocialQuestCompletedEvent {
            owner: profile.owner,
            quest_id,
            xp_awarded: xp_value,
            total_xp: profile.xp,
            new_tier: profile.tier,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}

/// Helper function to compute rank tiers from earned XP
fn calculate_tier(xp: u64) -> u8 {
    match xp {
        0..=99 => 1,       // Novice
        100..=499 => 2,    // Scavenger
        500..=1999 => 3,   // Sweeper
        2000..=4999 => 4,  // Harvester
        _ => 5,            // Void Sovereign
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION STRUCTS
// -----------------------------------------------------------------------------

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
        payer = owner,
        space = 8 + UserProfile::LEN,
        seeds = [USER_PROFILE_SEED, owner.key().as_ref()],
        bump
    )]
    pub user_profile: Account<'info, UserProfile>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SweepAndCloseAccount<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [USER_PROFILE_SEED, owner.key().as_ref()],
        bump = user_profile.bump,
        has_one = owner @ SweeperError::UnauthorizedProfileOwner
    )]
    pub user_profile: Account<'info, UserProfile>,

    #[account(mut)]
    pub token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub owner: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct VerifyAndCreditQuest<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [USER_PROFILE_SEED, user_profile.owner.as_ref()],
        bump = user_profile.bump
    )]
    pub user_profile: Account<'info, UserProfile>,

    pub oracle_signer: Signer<'info>,
}

// -----------------------------------------------------------------------------
// STORAGE LAYOUTS
// -----------------------------------------------------------------------------

#[account]
pub struct ProtocolConfig {
    pub admin: Pubkey,                   // 32
    pub oracle_authority: Pubkey,        // 32
    pub fee_basis_points: u16,           // 2
    pub total_rent_reclaimed_lamports: u64, // 8
    pub total_dust_accounts_closed: u64, // 8
    pub bump: u8,                        // 1
}

impl ProtocolConfig {
    pub const LEN: usize = 32 + 32 + 2 + 8 + 8 + 1;
}

#[account]
pub struct UserProfile {
    pub owner: Pubkey,                   // 32
    pub xp: u64,                         // 8
    pub tier: u8,                        // 1
    pub accounts_closed: u32,            // 4
    pub rent_reclaimed_lamports: u64,    // 8
    pub quests_completed_mask: u64,      // 8 (supports up to 64 discrete quests)
    pub bump: u8,                        // 1
}

impl UserProfile {
    pub const LEN: usize = 32 + 8 + 1 + 4 + 8 + 8 + 1;
}

// -----------------------------------------------------------------------------
// EVENTS
// -----------------------------------------------------------------------------

#[event]
pub struct ProtocolInitializedEvent {
    pub admin: Pubkey,
    pub oracle_authority: Pubkey,
    pub fee_basis_points: u16,
    pub timestamp: i64,
}

#[event]
pub struct UserProfileCreatedEvent {
    pub owner: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct AccountSweptEvent {
    pub owner: Pubkey,
    pub token_account: Pubkey,
    pub rent_reclaimed: u64,
    pub xp_awarded: u64,
    pub new_total_xp: u64,
    pub new_tier: u8,
    pub timestamp: i64,
}

#[event]
pub struct SocialQuestCompletedEvent {
    pub owner: Pubkey,
    pub quest_id: u8,
    pub xp_awarded: u64,
    pub total_xp: u64,
    pub new_tier: u8,
    pub timestamp: i64,
}

// -----------------------------------------------------------------------------
// CUSTOM ERRORS
// -----------------------------------------------------------------------------

#[error_code]
pub enum SweeperError {
    #[msg("Protocol fee exceeds maximum allowed threshold (1000 bps).")]
    FeeTooHigh,
    #[msg("Token account is not empty. Burn or swap remaining balance before closing.")]
    TokenAccountNotEmpty,
    #[msg("Signer does not own the target token account.")]
    UnauthorizedTokenOwner,
    #[msg("Target token account does not hold refundable rent lamports.")]
    ZeroRentAvailable,
    #[msg("Quest ID is out of bounds (0-63).")]
    InvalidQuestId,
    #[msg("XP value is invalid.")]
    InvalidXpValue,
    #[msg("Quest has already been claimed by this profile.")]
    QuestAlreadyCompleted,
    #[msg("Signer is not authorized as the quest attestation oracle.")]
    InvalidOracleAuthority,
    #[msg("Profile does not belong to the transaction signer.")]
    UnauthorizedProfileOwner,
}
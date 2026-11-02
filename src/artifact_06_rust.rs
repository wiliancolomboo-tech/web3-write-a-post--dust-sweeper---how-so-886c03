use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, CloseAccount, Token, TokenAccount, Mint};
use solana_program::ed25519_program;
use solana_program::instruction::Instruction;
use solana_program::sysvar::instructions as ix_sysvar;

declare_id!("SweepXPRenT11111111111111111111111111111111");

pub const CONFIG_SEED: &[u8] = b"global_config";
pub const USER_PROFILE_SEED: &[u8] = b"user_profile";
pub const MAX_ACTIVE_QUEST_INDEX: u16 = 256;

#[program]
pub mod dust_sweeper {
    use super::*;

    /// Initialize the global configuration and register the trusted attestation oracle.
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        attestation_oracle: Pubkey,
        fee_basis_points: u16,
    ) -> Result<()> {
        require!(fee_basis_points <= 1000, CustomError::FeeTooHigh); // Max 10%
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.attestation_oracle = attestation_oracle;
        config.fee_basis_points = fee_basis_points;
        config.total_accounts_closed = 0;
        config.total_lamports_reclaimed = 0;
        config.total_xp_awarded = 0;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    /// Update configuration parameters (Oracle key, protocol fee).
    pub fn update_protocol(
        ctx: Context<UpdateProtocol>,
        new_oracle: Option<Pubkey>,
        new_fee_basis_points: Option<u16>,
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;
        if let Some(oracle) = new_oracle {
            config.attestation_oracle = oracle;
        }
        if let Some(fee) = new_fee_basis_points {
            require!(fee <= 1000, CustomError::FeeTooHigh);
            config.fee_basis_points = fee;
        }
        Ok(())
    }

    /// Initializes a user's on-chain gamified XP profile.
    pub fn initialize_user_profile(ctx: Context<InitializeUserProfile>) -> Result<()> {
        let profile = &mut ctx.accounts.user_profile;
        profile.owner = ctx.accounts.user.key();
        profile.xp = 0;
        profile.level = 1;
        profile.dust_accounts_closed = 0;
        profile.quests_completed_bitmask = [0u8; 32]; // Supports up to 256 unique social quests
        profile.bump = ctx.bumps.user_profile;
        Ok(())
    }

    /// Sweeps residual dust tokens by burning them and reclaiming the rent SOL to user.
    pub fn sweep_and_close_account(
        ctx: Context<SweepAndCloseAccount>,
        burn_all_remaining: bool,
    ) -> Result<()> {
        let token_account = &ctx.accounts.token_account;
        let amount = token_account.amount;

        // 1. If balance exists and burn permission granted, execute burn CPI
        if amount > 0 {
            require!(burn_all_remaining, CustomError::ResidualBalanceNotEmpty);
            let burn_ctx = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.token_account.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            );
            token::burn(burn_ctx, amount)?;
        }

        // 2. Fetch starting lamport balance to measure actual reclaimed rent
        let reclaimed_lamports = ctx.accounts.token_account.to_account_info().lamports();

        // 3. Execute CloseAccount CPI to return rent to the owner
        let close_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.token_account.to_account_info(),
                destination: ctx.accounts.owner.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        );
        token::close_account(close_ctx)?;

        // 4. Update Protocol-Wide and User-Specific Metrics
        let config = &mut ctx.accounts.config;
        config.total_accounts_closed = config.total_accounts_closed.checked_add(1).unwrap();
        config.total_lamports_reclaimed = config
            .total_lamports_reclaimed
            .checked_add(reclaimed_lamports)
            .unwrap();

        let profile = &mut ctx.accounts.user_profile;
        profile.dust_accounts_closed = profile.dust_accounts_closed.checked_add(1).unwrap();
        
        // Award 50 bonus XP per dust account cleaned
        let bonus_xp: u64 = 50;
        profile.xp = profile.xp.checked_add(bonus_xp).unwrap();
        profile.recompute_level();

        config.total_xp_awarded = config.total_xp_awarded.checked_add(bonus_xp).unwrap();

        emit!(AccountSweptEvent {
            user: ctx.accounts.owner.key(),
            token_account: ctx.accounts.token_account.key(),
            mint: ctx.accounts.mint.key(),
            lamports_reclaimed: reclaimed_lamports,
            dust_burned: amount,
            xp_awarded: bonus_xp,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Verifies off-chain oracle Ed25519 signature of quest completion and credits on-chain XP.
    pub fn claim_social_quest_xp(
        ctx: Context<ClaimSocialQuestXP>,
        quest_id: u16,
        xp_reward: u32,
        oracle_signature: [u8; 64],
    ) -> Result<()> {
        require!(quest_id < MAX_ACTIVE_QUEST_INDEX, CustomError::InvalidQuestId);
        let profile = &mut ctx.accounts.user_profile;

        // Check if quest was already completed (bitmask check)
        let byte_index = (quest_id / 8) as usize;
        let bit_index = (quest_id % 8) as u8;
        let mask = 1u8 << bit_index;
        require!(
            (profile.quests_completed_bitmask[byte_index] & mask) == 0,
            CustomError::QuestAlreadyCompleted
        );

        // Introspect instructions sysvar to verify Ed25519 verification pre-instruction
        verify_ed25519_ix(
            &ctx.accounts.ix_sysvar.to_account_info(),
            &ctx.accounts.config.attestation_oracle.to_bytes(),
            &ctx.accounts.user.key().to_bytes(),
            quest_id,
            xp_reward,
            &oracle_signature,
        )?;

        // Set quest completion bit
        profile.quests_completed_bitmask[byte_index] |= mask;

        // Credit XP and check for level upgrades
        let xp_awarded = xp_reward as u64;
        profile.xp = profile.xp.checked_add(xp_awarded).unwrap();
        let old_level = profile.level;
        profile.recompute_level();

        let config = &mut ctx.accounts.config;
        config.total_xp_awarded = config.total_xp_awarded.checked_add(xp_awarded).unwrap();

        emit!(QuestCompletedEvent {
            user: ctx.accounts.user.key(),
            quest_id,
            xp_reward,
            new_level: profile.level,
            level_upgraded: profile.level > old_level,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}

/// Helper function inspecting Solana instructions sysvar to ensure Ed25519 program executed
fn verify_ed25519_ix(
    ix_sysvar_info: &AccountInfo,
    expected_oracle_pubkey: &[u8; 32],
    user_pubkey: &[u8; 32],
    quest_id: u16,
    xp_reward: u32,
    signature: &[u8; 64],
) -> Result<()> {
    let current_ix_index = ix_sysvar::load_current_index_checked(ix_sysvar_info)?;
    require!(current_ix_index > 0, CustomError::MissingPrecedingEd25519Instruction);

    // The Ed25519 verification instruction MUST precede this instruction in the same atomic transaction
    let prev_ix_index = (current_ix_index - 1) as u16;
    let instruction: Instruction = ix_sysvar::load_instruction_at_checked(
        prev_ix_index as usize,
        ix_sysvar_info,
    )?;

    // Verify program ID is the native ed25519 program
    require!(
        instruction.program_id == ed25519_program::id(),
        CustomError::InvalidEd25519Program
    );

    // Construct expected signed payload: [user_pubkey(32), quest_id(2), xp_reward(4)]
    let mut expected_message = Vec::with_capacity(38);
    expected_message.extend_from_slice(user_pubkey);
    expected_message.extend_from_slice(&quest_id.to_le_bytes());
    expected_message.extend_from_slice(&xp_reward.to_le_bytes());

    // Validate ed25519 data payload layout
    let data = instruction.data;
    require!(data.len() >= 112 + expected_message.len(), CustomError::MalformedEd25519Data);

    // Check public key match
    let pubkey_offset = u16::from_le_bytes([data[2], data[3]]) as usize;
    let pubkey_in_ix = &data[pubkey_offset..pubkey_offset + 32];
    require!(
        pubkey_in_ix == expected_oracle_pubkey,
        CustomError::AttestationAuthorityMismatch
    );

    // Check message content match
    let msg_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let msg_size = u16::from_le_bytes([data[4], data[5]]) as usize;
    let msg_in_ix = &data[msg_offset..msg_offset + msg_size];
    require!(
        msg_in_ix == expected_message.as_slice(),
        CustomError::AttestationMessageTampered
    );

    // Check signature match
    let sig_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let sig_in_ix = &data[sig_offset..sig_offset + 64];
    require!(
        sig_in_ix == signature,
        CustomError::InvalidSignatureData
    );

    Ok(())
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION STRUCTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub ছবিতে
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
pub struct UpdateProtocol<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = admin @ CustomError::Unauthorized
    )]
    pub config: Account<'info, GlobalConfig>,

    pub admin: Signer<'info>,
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
        seeds = [USER_PROFILE_SEED, owner.key().as_ref()],
        bump = user_profile.bump,
        has_one = owner @ CustomError::Unauthorized
    )]
    pub user_profile: Account<'info, UserProfile>,

    #[account(
        mut,
        constraint = token_account.owner == owner.key() @ CustomError::InvalidTokenAccountOwner,
        constraint = token_account.mint == mint.key() @ CustomError::MintMismatch
    )]
    pub token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub owner: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
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
        has_one = user @ CustomError::Unauthorized
    )]
    pub user_profile: Account<'info, UserProfile>,

    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: Validated in CPI using instructions sysvar
    #[account(address = ix_sysvar::ID)]
    pub ix_sysvar: AccountInfo<'info>,
}

// -----------------------------------------------------------------------------
// STATE DEFINITIONS
// -----------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct GlobalConfig {
    pub admin: Pubkey,
    pub attestation_oracle: Pubkey,
    pub fee_basis_points: u16,
    pub total_accounts_closed: u64,
    pub total_lamports_reclaimed: u64,
    pub total_xp_awarded: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct UserProfile {
    pub owner: Pubkey,
    pub xp: u64,
    pub level: u16,
    pub dust_accounts_closed: u64,
    pub quests_completed_bitmask: [u8; 32], // 256 bits = 256 quests
    pub bump: u8,
}

impl UserProfile {
    pub fn recompute_level(&mut self) {
        // Deterministic leveling curve: Level = 1 + floor(sqrt(XP / 100))
        let level = 1 + ((self.xp / 100) as f64).sqrt() as u16;
        self.level = level;
    }
}

// -----------------------------------------------------------------------------
// EVENTS
// -----------------------------------------------------------------------------

#[event]
pub struct AccountSweptEvent {
    pub user: Pubkey,
    pub token_account: Pubkey,
    pub mint: Pubkey,
    pub lamports_reclaimed: u64,
    pub dust_burned: u64,
    pub xp_awarded: u64,
    pub timestamp: i64,
}

#[event]
pub struct QuestCompletedEvent {
    pub user: Pubkey,
    pub quest_id: u16,
    pub xp_reward: u32,
    pub new_level: u16,
    pub level_upgraded: bool,
    pub timestamp: i64,
}

// -----------------------------------------------------------------------------
// ERROR CODES
// -----------------------------------------------------------------------------

#[error_code]
pub enum CustomError {
    #[msg("Fee basis points cannot exceed 1000 (10%).")]
    FeeTooHigh,
    #[msg("Account balance is non-zero and user did not confirm burning dust.")]
    ResidualBalanceNotEmpty,
    #[msg("Attestation oracle authority public key does not match.")]
    AttestationAuthorityMismatch,
    #[msg("Attestation payload does not match expected user/quest data.")]
    AttestationMessageTampered,
    #[msg("Instruction preceding this call must be ed25519 program verification.")]
    MissingPrecedingEd25519Instruction,
    #[msg("Target program is not the native Ed25519 Program.")]
    InvalidEd25519Program,
    #[msg("Ed25519 instruction data is malformed.")]
    MalformedEd25519Data,
    #[msg("Ed25519 signature payload is invalid.")]
    InvalidSignatureData,
    #[msg("Social quest has already been completed by this user.")]
    QuestAlreadyCompleted,
    #[msg("Specified quest ID is out of acceptable bounds.")]
    InvalidQuestId,
    #[msg("Unauthorized signer for account modification.")]
    Unauthorized,
    #[msg("Token account owner mismatch.")]
    InvalidTokenAccountOwner,
    #[msg("Token mint does not match account mint.")]
    MintMismatch,
}
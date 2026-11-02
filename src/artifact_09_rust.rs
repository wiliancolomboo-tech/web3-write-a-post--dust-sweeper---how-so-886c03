use anchor_lang::prelude::*;
use crate::errors::SweeperError;
use crate::state::{ProgramConfig, UserProfile};

#[derive(Accounts)]
pub struct CompleteSocialQuest<'info> {
    #[account(
        seeds = [b"program_config"],
        bump = config.bump
    )]
    pub config: Account<'info, ProgramConfig>,

    #[account(
        mut,
        seeds = [b"user_profile", user.key().as_ref()],
        bump = user_profile.bump,
        has_one = owner @ SweeperError::Unauthorized
    )]
    pub user_profile: Account<'info, UserProfile>,

    /// The user wallet claiming the quest reward
    pub owner: Signer<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    /// The off-chain oracle signer verifying quest completion
    pub oracle_signer: Signer<'info>,
}

pub fn handler(
    ctx: Context<CompleteSocialQuest>,
    quest_id: [u8; 32],
    xp_reward: u64,
) -> Result<()> {
    // 1. Verify that oracle signer matches the configured Quest Oracle
    require_keys_eq!(
        ctx.accounts.oracle_signer.key(),
        ctx.accounts.config.quest_oracle,
        SweeperError::Unauthorized
    );

    let profile = &mut ctx.accounts.user_profile;

    // 2. Prevent replay attacks of the exact same quest batch
    require!(
        profile.last_completed_quest_id != quest_id,
        SweeperError::QuestAlreadyClaimed
    );

    // 3. Award XP and update state
    profile.xp_points = profile
        .xp_points
        .checked_add(xp_reward)
        .ok_or(SweeperError::NumericalOverflow)?;

    profile.last_completed_quest_id = quest_id;
    profile.recalculate_tier();

    msg!(
        "Social Quest completed! Quest ID: {:?}, XP Awarded: {}, New Tier: {}",
        quest_id,
        xp_reward,
        profile.loyalty_tier
    );

    Ok(())
}
use anchor_lang::prelude::*;
use crate::state::UserProfile;

#[derive(Accounts)]
pub struct InitializeUser<'info> {
    #[account(
        init,
        payer = user,
        space = UserProfile::LEN,
        seeds = [b"user_profile", user.key().as_ref()],
        bump
    )]
    pub user_profile: Account<'info, UserProfile>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<InitializeUser>) -> Result<()> {
    let profile = &mut ctx.accounts.user_profile;
    profile.owner = ctx.accounts.user.key();
    profile.total_lamports_reclaimed = 0;
    profile.accounts_closed = 0;
    profile.xp_points = 100; // Registration welcome bonus XP
    profile.recalculate_tier();
    profile.last_completed_quest_id = [0u8; 32];
    profile.bump = ctx.bumps.user_profile;
    Ok(())
}
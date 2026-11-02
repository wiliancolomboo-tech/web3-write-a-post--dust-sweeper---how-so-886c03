use anchor_lang::prelude::*;

pub mod errors;
pub mod instructions;
pub mod state;

use instructions::close_empty_account::*;
use instructions::complete_social_quest::*;
use instructions::initialize_user::*;
use state::*;

declare_id!("DustSwp111111111111111111111111111111111111");

#[program]
pub mod dust_sweeper {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        quest_oracle: Pubkey,
        xp_per_close: u64,
    ) -> Result<()> {
        let cfg = &mut ctx.accounts.config;
        cfg.admin_authority = ctx.accounts.admin.key();
        cfg.quest_oracle = quest_oracle;
        cfg.xp_per_close = xp_per_close;
        cfg.bump = ctx.bumps.config;
        Ok(())
    }

    pub fn initialize_user(ctx: Context<InitializeUser>) -> Result<()> {
        instructions::initialize_user::handler(ctx)
    }

    pub fn close_empty_account(ctx: Context<CloseEmptyAccount>) -> Result<()> {
        instructions::close_empty_account::handler(ctx)
    }

    pub fn complete_social_quest(
        ctx: Context<CompleteSocialQuest>,
        quest_id: [u8; 32],
        xp_reward: u64,
    ) -> Result<()> {
        instructions::complete_social_quest::handler(ctx, quest_id, xp_reward)
    }
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(
        init,
        payer = admin,
        space = ProgramConfig::LEN,
        seeds = [b"program_config"],
        bump
    )]
    pub config: Account<'info, ProgramConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}
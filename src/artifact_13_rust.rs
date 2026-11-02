// programs/sol_cleanse/src/lib.rs
use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("Cleanse111111111111111111111111111111111111");

#[program]
pub mod sol_cleanse {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        base_xp_per_sweep: u64,
        protocol_fee_bps: u16,
    ) -> Result<()> {
        instructions::initialize_config::handler(ctx, base_xp_per_sweep, protocol_fee_bps)
    }

    pub fn initialize_profile(ctx: Context<InitializeProfile>) -> Result<()> {
        instructions::initialize_profile::handler(ctx)
    }

    pub fn sweep_dust(ctx: Context<SweepDust>, max_allowable_dust: u64) -> Result<()> {
        instructions::sweep_dust::handler(ctx, max_allowable_dust)
    }

    pub fn claim_social_quest(
        ctx: Context<ClaimSocialQuest>,
        quest_id: u8,
        xp_reward: u64,
        nonce: u64,
    ) -> Result<()> {
        instructions::claim_social_quest::handler(ctx, quest_id, xp_reward, nonce)
    }
}
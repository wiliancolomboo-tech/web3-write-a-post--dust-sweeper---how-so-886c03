use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("DustSwp111111111111111111111111111111111111");

#[program]
pub mod dust_sweeper {
    use super::*;

    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        protocol_fee_bps: u16,
    ) -> Result<()> {
        instructions::initialize::handler(ctx, protocol_fee_bps)
    }

    pub fn sweep_dust_and_close(
        ctx: Context<SweepDustAndClose>,
        max_dust_burnable: u64,
    ) -> Result<()> {
        instructions::sweep::handler(ctx, max_dust_burnable)
    }

    pub fn claim_social_quest_xp(
        ctx: Context<ClaimSocialQuestXp>,
        quest_id: u64,
        xp_reward: u64,
        nonce: u64,
        signature: [u8; 64],
    ) -> Result<()> {
        instructions::claim_quest::handler(ctx, quest_id, xp_reward, nonce, signature)
    }
}
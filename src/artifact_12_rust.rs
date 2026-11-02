use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("DustSwpQu3st111111111111111111111111111111111");

#[program]
pub mod dust_sweeper_quests {
    use super::*;

    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        oracle_signer: Pubkey,
        xp_per_sweep: u64,
    ) -> Result<()> {
        instructions::handle_init_protocol(ctx, oracle_signer, xp_per_sweep)
    }

    pub fn initialize_user_profile(ctx: Context<InitializeUserProfile>) -> Result<()> {
        instructions::handle_init_profile(ctx)
    }

    pub fn sweep_and_close_account(
        ctx: Context<SweepAndCloseDustAccount>,
        max_dust_burn_amount: u64,
    ) -> Result<()> {
        instructions::handle_sweep_account(ctx, max_dust_burn_amount)
    }

    pub fn claim_social_quest(
        ctx: Context<ClaimSocialQuest>,
        quest_id: u8,
        xp_reward: u64,
        nonce: u64,
        expiry_timestamp: i64,
        signature: [u8; 64],
    ) -> Result<()> {
        instructions::handle_claim_quest(
            ctx,
            quest_id,
            xp_reward,
            nonce,
            expiry_timestamp,
            signature,
        )
    }
}
use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("SwpCore1111111111111111111111111111111111111");

#[program]
pub mod solsweep_core {
    use super::*;

    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        base_fee_bps: u16,
    ) -> Result<()> {
        instructions::initialize_protocol::handler(ctx, base_fee_bps)
    }

    pub fn initialize_profile(ctx: Context<InitializeProfile>) -> Result<()> {
        instructions::initialize_profile::handler(ctx)
    }

    pub fn claim_quest(
        ctx: Context<ClaimQuest>,
        quest_id: u16,
        xp_amount: u64,
        nonce: u64,
        expiry_timestamp: i64,
    ) -> Result<()> {
        instructions::claim_quest::handler(ctx, quest_id, xp_amount, nonce, expiry_timestamp)
    }
}
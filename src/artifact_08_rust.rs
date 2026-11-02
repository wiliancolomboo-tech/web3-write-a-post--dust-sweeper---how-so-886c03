use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};
use crate::errors::SweeperError;
use crate::state::{ProgramConfig, UserProfile};

#[derive(Accounts)]
pub struct CloseEmptyAccount<'info> {
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

    #[account(
        mut,
        constraint = token_account.owner == user.key() @ SweeperError::InvalidAccountOwner,
        constraint = token_account.amount == 0 @ SweeperError::AccountBalanceNotZero
    )]
    pub token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<CloseEmptyAccount>) -> Result<()> {
    let rent_lamports = ctx.accounts.token_account.to_account_info().lamports();

    // 1. Invoke SPL Token program CPI to close account and return lamports to user
    let cpi_accounts = CloseAccount {
        account: ctx.accounts.token_account.to_account_info(),
        destination: ctx.accounts.user.to_account_info(),
        authority: ctx.accounts.owner.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    token::close_account(cpi_ctx)?;

    // 2. State updates: update XP and closed account count
    let profile = &mut ctx.accounts.user_profile;
    profile.total_lamports_reclaimed = profile
        .total_lamports_reclaimed
        .checked_add(rent_lamports)
        .ok_or(SweeperError::NumericalOverflow)?;

    profile.accounts_closed = profile
        .accounts_closed
        .checked_add(1)
        .ok_or(SweeperError::NumericalOverflow)?;

    let xp_earned = ctx.accounts.config.xp_per_close;
    profile.xp_points = profile
        .xp_points
        .checked_add(xp_earned)
        .ok_or(SweeperError::NumericalOverflow)?;

    profile.recalculate_tier();

    msg!(
        "Closed account: {}. Reclaimed {} lamports. New XP balance: {}",
        ctx.accounts.token_account.key(),
        rent_lamports,
        profile.xp_points
    );

    Ok(())
}
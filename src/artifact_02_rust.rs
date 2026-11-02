use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, CloseAccount, Token, TokenAccount};

declare_id!("SwpXP111111111111111111111111111111111111111");

#[program]
pub mod sweep_xp {
    use super::*;

    /// Initialize global protocol state and authority
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        fee_bps: u16,
        oracle_signer: Pubkey,
    ) -> Result<()> {
        require!(fee_bps <= 1000, SweepError::FeeTooHigh); // Max 10%
        let config = &mut ctx.accounts.protocol_config;
        config.authority = ctx.accounts.authority.key();
        config.fee_receiver = ctx.accounts.fee_receiver.key();
        config.oracle_signer = oracle_signer;
        config.fee_bps = fee_bps;
        config.total_swept_lamports = 0;
        config.total_accounts_closed = 0;
        config.bump = ctx.bumps.protocol_config;

        emit!(ProtocolInitializedEvent {
            authority: config.authority,
            fee_receiver: config.fee_receiver,
            oracle_signer,
            fee_bps,
        });

        Ok(())
    }

    /// Initialize user's persistent XP PDA
    pub fn initialize_user_xp(ctx: Context<InitializeUserXP>) -> Result<()> {
        let user_xp = &mut ctx.accounts.user_xp_state;
        user_xp.user = ctx.accounts.user.key();
        user_xp.total_xp = 0;
        user_xp.total_sol_recovered = 0;
        user_xp.accounts_closed = 0;
        user_xp.quests_completed = 0;
        user_xp.last_sweep_timestamp = 0;
        user_xp.nonce = 0;
        user_xp.bump = ctx.bumps.user_xp_state;

        emit!(UserXPInitializedEvent {
            user: user_xp.user,
        });

        Ok(())
    }

    /// Sweeps an untradeable/worthless token: Burns existing balance and closes account,
    /// reclaiming 100% of the rent to the user, and awards base XP.
    pub fn sweep_burn_and_close(
        ctx: Context<SweepBurnAndClose>,
        amount_to_burn: u64,
    ) -> Result<()> {
        let user = &ctx.accounts.user;
        let token_account = &ctx.accounts.token_account;

        // 1. Burn dust balance if remaining
        if amount_to_burn > 0 {
            let burn_ctx = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: token_account.to_account_info(),
                    authority: user.to_account_info(),
                },
            );
            token::burn(burn_ctx, amount_to_burn)?;
        }

        // 2. Close token account and route lamport rent back to user
        let starting_lamports = user.lamports();
        let close_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: token_account.to_account_info(),
                destination: user.to_account_info(),
                authority: user.to_account_info(),
            },
        );
        token::close_account(close_ctx)?;

        let ending_lamports = user.lamports();
        let recovered_rent = ending_lamports.saturating_sub(starting_lamports);

        // 3. Update protocol and user state
        let config = &mut ctx.accounts.protocol_config;
        config.total_accounts_closed = config.total_accounts_closed.checked_add(1).unwrap();
        config.total_swept_lamports = config.total_swept_lamports.checked_add(recovered_rent).unwrap();

        let user_xp = &mut ctx.accounts.user_xp_state;
        user_xp.accounts_closed = user_xp.accounts_closed.checked_add(1).unwrap();
        user_xp.total_sol_recovered = user_xp.total_sol_recovered.checked_add(recovered_rent).unwrap();
        user_xp.last_sweep_timestamp = Clock::get()?.unix_timestamp;

        // Base reward: 100 XP per closed ATA + 10 XP per 0.001 SOL recovered
        let xp_earned: u64 = 100 + (recovered_rent / 100_000);
        user_xp.total_xp = user_xp.total_xp.checked_add(xp_earned).unwrap();

        emit!(DustSweptEvent {
            user: user.key(),
            mint: ctx.accounts.mint.key(),
            rent_recovered: recovered_rent,
            xp_awarded: xp_earned,
            timestamp: user_xp.last_sweep_timestamp,
        });

        Ok(())
    }

    /// Claim verified off-chain social quest XP using an Ed25519 Oracle attestation
    pub fn claim_social_quest_xp(
        ctx: Context<ClaimSocialQuestXP>,
        quest_id: u64,
        xp_amount: u32,
        oracle_signature: [u8; 64],
    ) -> Result<()> {
        let config = &ctx.accounts.protocol_config;
        let user_xp = &mut ctx.accounts.user_xp_state;

        // Prevent replay attacks using monotonic nonce
        let expected_nonce = user_xp.nonce;

        // Construct attestation message: user_pubkey (32) + quest_id (8) + xp_amount (4) + nonce (8)
        let mut message = Vec::with_capacity(52);
        message.extend_from_slice(user_xp.user.as_ref());
        message.extend_from_slice(&quest_id.to_le_bytes());
        message.extend_from_slice(&xp_amount.to_le_bytes());
        message.extend_from_slice(&expected_nonce.to_le_bytes());

        // Verify ed25519 instruction signature
        verify_oracle_signature(
            &config.oracle_signer,
            &message,
            &oracle_signature,
            &ctx.accounts.instruction_sysvar,
        )?;

        // Update state
        user_xp.total_xp = user_xp.total_xp.checked_add(xp_amount as u64).unwrap();
        user_xp.quests_completed = user_xp.quests_completed.checked_add(1).unwrap();
        user_xp.nonce = user_xp.nonce.checked_add(1).unwrap();

        emit!(SocialQuestCompletedEvent {
            user: user_xp.user,
            quest_id,
            xp_awarded: xp_amount,
            new_total_xp: user_xp.total_xp,
            nonce: user_xp.nonce,
        });

        Ok(())
    }
}

/// Helper function to introspect transaction instructions and verify Ed25519 signature
fn verify_oracle_signature(
    expected_oracle: &Pubkey,
    expected_message: &[u8],
    provided_signature: &[u8; 64],
    instruction_sysvar_account: &AccountInfo,
) -> Result<()> {
    let current_instruction_index = anchor_lang::solana_program::sysvar::instructions::load_current_index_checked(
        instruction_sysvar_account,
    )?;

    // The immediately preceding instruction must be the Ed25519Program verification
    require!(current_instruction_index > 0, SweepError::MissingSignatureVerificationInstruction);
    let ed25519_instruction_index = current_instruction_index - 1;

    let ed25519_instruction = anchor_lang::solana_program::sysvar::instructions::load_instruction_at_checked(
        ed25519_instruction_index as usize,
        instruction_sysvar_account,
    )?;

    // Ensure instruction is from standard ed25519 program
    require!(
        ed25519_instruction.program_id == anchor_lang::solana_program::ed25519_program::id(),
        SweepError::InvalidEd25519Program
    );

    let data = &ed25519_instruction.data;
    require!(data.len() >= 16 + 64 + 32 + expected_message.len(), SweepError::MalformedSignatureInstruction);

    // Verify signer pubkey matches configured oracle
    let pubkey_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let signer_bytes = &data[pubkey_offset..pubkey_offset + 32];
    require!(signer_bytes == expected_oracle.as_ref(), SweepError::UnauthorizedOracle);

    // Verify signature matches
    let sig_offset = u16::from_le_bytes([data[2], data[3]]) as usize;
    let sig_bytes = &data[sig_offset..sig_offset + 64];
    require!(sig_bytes == provided_signature, SweepError::InvalidSignatureData);

    // Verify message payload matches
    let msg_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let msg_size = u16::from_le_bytes([data[12], data[13]]) as usize;
    require!(msg_size == expected_message.len(), SweepError::MessageSizeMismatch);
    require!(&data[msg_offset..msg_offset + msg_size] == expected_message, SweepError::MessageContentMismatch);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + ProtocolConfig::LEN,
        seeds = [b"config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: Fee receiver account
    pub fee_receiver: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeUserXP<'info> {
    #[account(
        init,
        payer = user,
        space = 8 + UserXPState::LEN,
        seeds = [b"xp_state", user.key().as_ref()],
        bump
    )]
    pub user_xp_state: Account<'info, UserXPState>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SweepBurnAndClose<'info> {
    #[account(
        mut,
        seeds = [b"config"],
        bump = protocol_config.bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
    #[account(
        mut,
        seeds = [b"xp_state", user.key().as_ref()],
        bump = user_xp_state.bump,
        has_one = user
    )]
    pub user_xp_state: Account<'info, UserXPState>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        mut,
        constraint = token_account.owner == user.key() @ SweepError::InvalidAccountOwner,
        constraint = token_account.mint == mint.key() @ SweepError::MintMismatch
    )]
    pub token_account: Account<'info, TokenAccount>,
    /// CHECK: Validated against token_account
    #[account(mut)]
    pub mint: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ClaimSocialQuestXP<'info> {
    #[account(
        seeds = [b"config"],
        bump = protocol_config.bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
    #[account(
        mut,
        seeds = [b"xp_state", user.key().as_ref()],
        bump = user_xp_state.bump,
        has_one = user
    )]
    pub user_xp_state: Account<'info, UserXPState>,
    pub user: Signer<'info>,
    /// CHECK: sysvar instructions account for cryptographic introspection
    #[account(address = anchor_lang::solana_program::sysvar::instructions::id())]
    pub instruction_sysvar: AccountInfo<'info>,
}

#[account]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub fee_receiver: Pubkey,
    pub oracle_signer: Pubkey,
    pub fee_bps: u16,
    pub total_swept_lamports: u64,
    pub total_accounts_closed: u64,
    pub bump: u8,
}

impl ProtocolConfig {
    pub const LEN: usize = 32 + 32 + 32 + 2 + 8 + 8 + 1;
}

#[account]
pub struct UserXPState {
    pub user: Pubkey,
    pub total_xp: u64,
    pub total_sol_recovered: u64,
    pub accounts_closed: u64,
    pub quests_completed: u32,
    pub last_sweep_timestamp: i64,
    pub nonce: u64,
    pub bump: u8,
}

impl UserXPState {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 4 + 8 + 8 + 1;
}

#[event]
pub struct ProtocolInitializedEvent {
    pub authority: Pubkey,
    pub fee_receiver: Pubkey,
    pub oracle_signer: Pubkey,
    pub fee_bps: u16,
}

#[event]
pub struct UserXPInitializedEvent {
    pub user: Pubkey,
}

#[event]
pub struct DustSweptEvent {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub rent_recovered: u64,
    pub xp_awarded: u64,
    pub timestamp: i64,
}

#[event]
pub struct SocialQuestCompletedEvent {
    pub user: Pubkey,
    pub quest_id: u64,
    pub xp_awarded: u32,
    pub new_total_xp: u64,
    pub nonce: u64,
}

#[error_code]
pub enum SweepError {
    #[msg("Fee basis points cannot exceed 1000 (10%)")]
    FeeTooHigh,
    #[msg("Account does not belong to the transaction signer")]
    InvalidAccountOwner,
    #[msg("Mint address does not match token account mint")]
    MintMismatch,
    #[msg("Preceding instruction is not the Ed25519 signature verification")]
    MissingSignatureVerificationInstruction,
    #[msg("Invalid program ID for signature verification")]
    InvalidEd25519Program,
    #[msg("Signature instruction data is malformed")]
    MalformedSignatureInstruction,
    #[msg("Oracle pubkey does not match authorized protocol signer")]
    UnauthorizedOracle,
    #[msg("Invalid signature bytes")]
    InvalidSignatureData,
    #[msg("Message size mismatch in signature verification")]
    MessageSizeMismatch,
    #[msg("Message content mismatch in signature verification")]
    MessageContentMismatch,
}
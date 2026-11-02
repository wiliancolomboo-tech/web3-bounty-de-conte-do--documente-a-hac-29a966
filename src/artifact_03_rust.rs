use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("InstaSoL11111111111111111111111111111111111");

pub const MAX_POST_ID_LEN: usize = 32;
pub const MAX_HANDLE_LEN: usize = 32;
pub const DISCRIMINATOR_LENGTH: usize = 8;

#[program]
pub mod instasol_engagement {
    use super::*;

    /// Initializes a hackathon event instance with a dedicated escrow vault.
    pub fn initialize_event(
        ctx: Context<InitializeEvent>,
        event_id: String,
        bounty_per_claim: u64,
        min_engagement_threshold: u32,
        end_slot: u64,
    ) -> Result<()> {
        require!(event_id.len() <= 32, InstaSolError::EventIdTooLong);
        require!(bounty_per_claim > 0, InstaSolError::InvalidBountyAmount);
        require!(end_slot > Clock::get()?.slot, InstaSolError::InvalidExpiry);

        let event = &mut ctx.accounts.event_config;
        event.authority = ctx.accounts.authority.key();
        event.oracle_pubkey = ctx.accounts.oracle_pubkey.key();
        event.token_mint = ctx.accounts.reward_mint.key();
        event.bounty_per_claim = bounty_per_claim;
        event.min_engagement_threshold = min_engagement_threshold;
        event.end_slot = end_slot;
        event.total_submissions = 0;
        event.total_payout = 0;
        event.bump = ctx.bumps.event_config;
        event.vault_bump = ctx.bumps.bounty_vault;
        event.event_id = event_id;

        emit!(EventInitialized {
            event_key: event.key(),
            authority: event.authority,
            bounty_per_claim,
            end_slot,
        });

        Ok(())
    }

    /// Deposits bounty escrow funds into the event vault.
    pub fn fund_event_escrow(ctx: Context<FundEscrow>, amount: u64) -> Result<()> {
        require!(amount > 0, InstaSolError::ZeroTransferAmount);

        let cpi_accounts = TransferChecked {
            from: ctx.accounts.funder_token_account.to_account_info(),
            mint: ctx.accounts.reward_mint.to_account_info(),
            to: ctx.accounts.bounty_vault.to_account_info(),
            authority: ctx.accounts.funder.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        token::transfer_checked(cpi_ctx, amount, ctx.accounts.reward_mint.decimals)?;

        emit!(EscrowFunded {
            event_key: ctx.accounts.event_config.key(),
            amount,
        });

        Ok(())
    }

    /// Verifies Instagram documentation content via an Oracle signature validation,
    /// marks the submission as claimed, and transfers the SPL bounty to the creator.
    pub fn verify_and_claim_submission(
        ctx: Context<VerifyAndClaimSubmission>,
        post_id: String,
        instagram_handle: String,
        content_sha256: [u8; 32],
        engagement_metric: u32,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let event = &mut ctx.accounts.event_config;

        // Security Validations
        require!(clock.slot <= event.end_slot, InstaSolError::EventExpired);
        require!(post_id.len() <= MAX_POST_ID_LEN, InstaSolError::PostIdTooLong);
        require!(instagram_handle.len() <= MAX_HANDLE_LEN, InstaSolError::HandleTooLong);
        require!(
            engagement_metric >= event.min_engagement_threshold,
            InstaSolError::EngagementBelowThreshold
        );

        let vault_balance = ctx.accounts.bounty_vault.amount;
        require!(
            vault_balance >= event.bounty_per_claim,
            InstaSolError::InsufficientEscrowBalance
        );

        // Populate submission PDA record
        let submission = &mut ctx.accounts.submission_record;
        submission.creator = ctx.accounts.creator.key();
        submission.event = event.key();
        submission.content_sha256 = content_sha256;
        submission.claimed_slot = clock.slot;
        submission.engagement_score = engagement_metric;
        submission.bump = ctx.bumps.submission_record;

        // Increment event-wide metrics
        event.total_submissions = event
            .total_submissions
            .checked_add(1)
            .ok_or(InstaSolError::MathOverflow)?;
        event.total_payout = event
            .total_payout
            .checked_add(event.bounty_per_claim)
            .ok_or(InstaSolError::MathOverflow)?;

        // Execute CPI token disbursal using event seeds
        let event_id_bytes = event.event_id.as_bytes();
        let event_bump = [event.bump];
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"event",
            event_id_bytes,
            &event_bump,
        ]];

        let cpi_accounts = TransferChecked {
            from: ctx.accounts.bounty_vault.to_account_info(),
            mint: ctx.accounts.reward_mint.to_account_info(),
            to: ctx.accounts.creator_token_account.to_account_info(),
            authority: ctx.accounts.event_config.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);

        token::transfer_checked(cpi_ctx, event.bounty_per_claim, ctx.accounts.reward_mint.decimals)?;

        emit!(SubmissionClaimed {
            event_key: event.key(),
            creator: ctx.accounts.creator.key(),
            post_id,
            engagement_metric,
            payout_amount: event.bounty_per_claim,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Account Contexts
// -----------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(event_id: String)]
pub struct InitializeEvent<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Trusted off-chain oracle service verifying Instagram Graph signatures
    pub oracle_pubkey: AccountInfo<'info>,

    pub reward_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        space = DISCRIMINATOR_LENGTH + EventConfig::INIT_SPACE,
        seeds = [b"event", event_id.as_bytes()],
        bump
    )]
    pub event_config: Account<'info, EventConfig>,

    #[account(
        init,
        payer = authority,
        seeds = [b"vault", event_config.key().as_ref()],
        bump,
        token::mint = reward_mint,
        token::authority = event_config,
    )]
    pub bounty_vault: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct FundEscrow<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,

    #[account(mut)]
    pub funder_token_account: Account<'info, TokenAccount>,

    #[account(
        seeds = [b"event", event_config.event_id.as_bytes()],
        bump = event_config.bump
    )]
    pub event_config: Account<'info, EventConfig>,

    pub reward_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"vault", event_config.key().as_ref()],
        bump = event_config.vault_bump,
        token::mint = reward_mint,
        token::authority = event_config,
    )]
    pub bounty_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(post_id: String, instagram_handle: String, content_sha256: [u8; 32])]
pub struct VerifyAndClaimSubmission<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    /// Autonomous Instagram verification keeper
    #[account(
        constraint = oracle_signer.key() == event_config.oracle_pubkey @ InstaSolError::UnauthorizedOracle
    )]
    pub oracle_signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"event", event_config.event_id.as_bytes()],
        bump = event_config.bump
    )]
    pub event_config: Account<'info, EventConfig>,

    #[account(
        init,
        payer = creator,
        space = DISCRIMINATOR_LENGTH + SubmissionRecord::INIT_SPACE,
        seeds = [
            b"submission",
            event_config.key().as_ref(),
            anchor_lang::solana_program::hash::hash(post_id.as_bytes()).as_ref()
        ],
        bump
    )]
    pub submission_record: Account<'info, SubmissionRecord>,

    pub reward_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"vault", event_config.key().as_ref()],
        bump = event_config.vault_bump,
        token::mint = reward_mint,
        token::authority = event_config,
    )]
    pub bounty_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = reward_mint,
        token::authority = creator,
    )]
    pub creator_token_account: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

// -----------------------------------------------------------------------------
// State Structures
// -----------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct EventConfig {
    pub authority: Pubkey,
    pub oracle_pubkey: Pubkey,
    pub token_mint: Pubkey,
    pub bounty_per_claim: u64,
    pub total_payout: u64,
    pub total_submissions: u64,
    pub end_slot: u64,
    pub min_engagement_threshold: u32,
    pub bump: u8,
    pub vault_bump: u8,
    #[max_len(32)]
    pub event_id: String,
}

#[account]
#[derive(InitSpace)]
pub struct SubmissionRecord {
    pub creator: Pubkey,
    pub event: Pubkey,
    pub content_sha256: [u8; 32],
    pub claimed_slot: u64,
    pub engagement_score: u32,
    pub bump: u8,
}

// -----------------------------------------------------------------------------
// Events & Errors
// -----------------------------------------------------------------------------

#[event]
pub struct EventInitialized {
    pub event_key: Pubkey,
    pub authority: Pubkey,
    pub bounty_per_claim: u64,
    pub end_slot: u64,
}

#[event]
pub struct EscrowFunded {
    pub event_key: Pubkey,
    pub amount: u64,
}

#[event]
pub struct SubmissionClaimed {
    pub event_key: Pubkey,
    pub creator: Pubkey,
    pub post_id: String,
    pub engagement_metric: u32,
    pub payout_amount: u64,
}

#[error_code]
pub enum InstaSolError {
    #[msg("Event ID exceeds maximum length of 32 characters")]
    EventIdTooLong,
    #[msg("Post ID exceeds maximum length of 32 characters")]
    PostIdTooLong,
    #[msg("Handle exceeds maximum length of 32 characters")]
    HandleTooLong,
    #[msg("Bounty amount must be greater than zero")]
    InvalidBountyAmount,
    #[msg("Event expiry slot must be in the future")]
    InvalidExpiry,
    #[msg("Escrow fund amount cannot be zero")]
    ZeroTransferAmount,
    #[msg("Event has reached its ending slot limit")]
    EventExpired,
    #[msg("Engagement metrics are lower than the event minimum threshold")]
    EngagementBelowThreshold,
    #[msg("Escrow vault has insufficient funds for bounty payout")]
    InsufficientEscrowBalance,
    #[msg("Unauthorized verification oracle signature")]
    UnauthorizedOracle,
    #[msg("Mathematical calculation overflow")]
    MathOverflow,
}
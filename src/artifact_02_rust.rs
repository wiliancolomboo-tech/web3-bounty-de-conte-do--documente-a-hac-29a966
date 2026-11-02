use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("InstaHack11111111111111111111111111111111111");

#[program]
pub mod instahack_sol {
    use super::*;

    /// Initializes a hackathon content bounty campaign with native SOL escrow
    pub fn initialize_event(
        ctx: Context<InitializeEvent>,
        event_id: String,
        bounty_per_approved_post: u64,
        max_submissions_per_creator: u8,
        start_timestamp: i64,
        end_timestamp: i64,
        oracle_pubkey: Pubkey,
    ) -> Result<()> {
        require!(event_id.len() <= 32, ErrorCode::EventIdTooLong);
        require!(start_timestamp < end_timestamp, ErrorCode::InvalidTimestampRange);
        require!(bounty_per_approved_post > 0, ErrorCode::ZeroBountyAmount);

        let event = &mut ctx.accounts.event;
        event.admin = ctx.accounts.admin.key();
        event.oracle = oracle_pubkey;
        event.event_id = event_id;
        event.bounty_per_approved_post = bounty_per_approved_post;
        event.max_submissions_per_creator = max_submissions_per_creator;
        event.start_timestamp = start_timestamp;
        event.end_timestamp = end_timestamp;
        event.total_submissions = 0;
        event.total_payout = 0;
        event.is_active = true;
        event.vault_bump = ctx.bumps.vault;

        Ok(())
    }

    /// Deposits funds into the event escrow vault
    pub fn fund_escrow(ctx: Context<FundEscrow>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::ZeroDepositAmount);

        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        );
        transfer(cpi_context, amount)?;

        emit!(EscrowFundedEvent {
            event_id: ctx.accounts.event.event_id.clone(),
            funder: ctx.accounts.funder.key(),
            amount,
        });

        Ok(())
    }

    /// Registers a student creator under a specific hackathon event
    pub fn register_creator(
        ctx: Context<RegisterCreator>,
        instagram_handle_hash: [u8; 32],
    ) -> Result<()> {
        let creator_record = &mut ctx.accounts.creator_record;
        creator_record.creator = ctx.accounts.creator.key();
        creator_record.event = ctx.accounts.event.key();
        creator_record.instagram_handle_hash = instagram_handle_hash;
        creator_record.submission_count = 0;
        creator_record.total_earned = 0;
        creator_record.bump = ctx.bumps.creator_record;

        Ok(())
    }

    /// Submits Instagram media proof, verified on-chain and dispatches bounty
    pub fn submit_content_proof(
        ctx: Context<SubmitContentProof>,
        media_id_hash: [u8; 32],
        content_sha256: [u8; 32],
        post_timestamp: i64,
        media_type: u8, // 0: Post, 1: Reel, 2: Carousel
    ) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        let event = &ctx.accounts.event;

        // Security guards
        require!(event.is_active, ErrorCode::EventInactive);
        require!(
            current_time >= event.start_timestamp && current_time <= event.end_timestamp,
            ErrorCode::EventOutsideActiveWindow
        );
        require!(
            post_timestamp >= event.start_timestamp && post_timestamp <= event.end_timestamp,
            ErrorCode::InvalidPostTimestamp
        );

        let creator_record = &mut ctx.accounts.creator_record;
        require!(
            creator_record.submission_count < event.max_submissions_per_creator,
            ErrorCode::MaxSubmissionsExceeded
        );

        // Populate submission PDA
        let submission = &mut ctx.accounts.submission;
        submission.creator = ctx.accounts.creator.key();
        submission.event = event.key();
        submission.media_id_hash = media_id_hash;
        submission.content_sha256 = content_sha256;
        submission.post_timestamp = post_timestamp;
        submission.media_type = media_type;
        submission.verified_at = current_time;
        submission.payout_amount = event.bounty_per_approved_post;
        submission.bump = ctx.bumps.submission;

        // Update creator state
        creator_record.submission_count = creator_record
            .submission_count
            .checked_add(1)
            .ok_or(ErrorCode::MathOverflow)?;
        creator_record.total_earned = creator_record
            .total_earned
            .checked_add(event.bounty_per_approved_post)
            .ok_or(ErrorCode::MathOverflow)?;

        // Check vault solvency
        let vault = &ctx.accounts.vault;
        let vault_lamports = vault.to_account_info().lamports();
        require!(
            vault_lamports >= event.bounty_per_approved_post,
            ErrorCode::EscrowInsufficientFunds
        );

        // Transfer funds from PDA vault to Creator
        let event_key = event.key();
        let seeds = &[
            b"vault",
            event_key.as_ref(),
            &[event.vault_bump],
        ];
        let signer_seeds = &[&seeds[..]];

        **vault.to_account_info().try_borrow_mut_lamports()? = vault
            .to_account_info()
            .lamports()
            .checked_sub(event.bounty_per_approved_post)
            .ok_or(ErrorCode::MathOverflow)?;

        **ctx.accounts.creator.to_account_info().try_borrow_mut_lamports()? = ctx
            .accounts
            .creator
            .to_account_info()
            .lamports()
            .checked_add(event.bounty_per_approved_post)
            .ok_or(ErrorCode::MathOverflow)?;

        // Update event globals
        let event_mut = &mut ctx.accounts.event;
        event_mut.total_submissions = event_mut
            .total_submissions
            .checked_add(1)
            .ok_or(ErrorCode::MathOverflow)?;
        event_mut.total_payout = event_mut
            .total_payout
            .checked_add(event.bounty_per_approved_post)
            .ok_or(ErrorCode::MathOverflow)?;

        emit!(ContentVerifiedAndPaidEvent {
            event_id: event_mut.event_id.clone(),
            creator: ctx.accounts.creator.key(),
            media_id_hash,
            payout: event_mut.bounty_per_approved_post,
        });

        Ok(())
    }

    /// Admin reclamation of leftover escrow funds after event concludes
    pub fn close_event(ctx: Context<CloseEvent>) -> Result<()> {
        let clock = Clock::get()?;
        let event = &ctx.accounts.event;

        require!(
            clock.unix_timestamp > event.end_timestamp || !event.is_active,
            ErrorCode::EventStillRunning
        );

        let vault_lamports = ctx.accounts.vault.to_account_info().lamports();
        if vault_lamports > 0 {
            **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? = 0;
            **ctx.accounts.admin.to_account_info().try_borrow_mut_lamports()? = ctx
                .accounts
                .admin
                .to_account_info()
                .lamports()
                .checked_add(vault_lamports)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        let event_mut = &mut ctx.accounts.event;
        event_mut.is_active = false;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Account Context Structs
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(event_id: String)]
pub struct InitializeEvent<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + HackathonEvent::INIT_SPACE,
        seeds = [b"event", event_id.as_bytes()],
        bump
    )]
    pub event: Account<'info, HackathonEvent>,

    /// CHECK: Native SOL vault PDA authorized to hold and distribute lamports
    #[account(
        mut,
        seeds = [b"vault", event.key().as_ref()],
        bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FundEscrow<'info> {
    #[account(mut)]
    pub event: Account<'info, HackathonEvent>,

    /// CHECK: Validated by PDA seeds against event
    #[account(
        mut,
        seeds = [b"vault", event.key().as_ref()],
        bump = event.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(mut)]
    pub funder: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RegisterCreator<'info> {
    #[account(
        init,
        payer = creator,
        space = 8 + CreatorRecord::INIT_SPACE,
        seeds = [b"creator", event.key().as_ref(), creator.key().as_ref()],
        bump
    )]
    pub creator_record: Account<'info, CreatorRecord>,

    pub event: Account<'info, HackathonEvent>,

    #[account(mut)]
    pub creator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(media_id_hash: [u8; 32])]
pub struct SubmitContentProof<'info> {
    #[account(mut)]
    pub event: Account<'info, HackathonEvent>,

    /// CHECK: PDA vault holding SOL rewards
    #[account(
        mut,
        seeds = [b"vault", event.key().as_ref()],
        bump = event.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [b"creator", event.key().as_ref(), creator.key().as_ref()],
        bump = creator_record.bump,
        has_one = creator
    )]
    pub creator_record: Account<'info, CreatorRecord>,

    #[account(
        init,
        payer = oracle,
        space = 8 + ContentSubmission::INIT_SPACE,
        seeds = [b"submission", event.key().as_ref(), media_id_hash.as_ref()],
        bump
    )]
    pub submission: Account<'info, ContentSubmission>,

    /// The student creator receiving rewards
    /// CHECK: Validated against creator_record.creator
    #[account(mut, address = creator_record.creator)]
    pub creator: SystemAccount<'info>,

    /// Verified Oracle daemon co-signing the valid Instagram post
    #[account(
        mut,
        constraint = oracle.key() == event.oracle @ ErrorCode::UnauthorizedOracle
    )]
    pub oracle: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CloseEvent<'info> {
    #[account(
        mut,
        has_one = admin @ ErrorCode::UnauthorizedAdmin
    )]
    pub event: Account<'info, HackathonEvent>,

    /// CHECK: Vault matching event
    #[account(
        mut,
        seeds = [b"vault", event.key().as_ref()],
        bump = event.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,
}

// ---------------------------------------------------------------------------
// Account State Definitions
// ---------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct HackathonEvent {
    pub admin: Pubkey,
    pub oracle: Pubkey,
    #[max_len(32)]
    pub event_id: String,
    pub bounty_per_approved_post: u64,
    pub max_submissions_per_creator: u8,
    pub start_timestamp: i64,
    pub end_timestamp: i64,
    pub total_submissions: u64,
    pub total_payout: u64,
    pub is_active: bool,
    pub vault_bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct CreatorRecord {
    pub creator: Pubkey,
    pub event: Pubkey,
    pub instagram_handle_hash: [u8; 32],
    pub submission_count: u8,
    pub total_earned: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct ContentSubmission {
    pub creator: Pubkey,
    pub event: Pubkey,
    pub media_id_hash: [u8; 32],
    pub content_sha256: [u8; 32],
    pub post_timestamp: i64,
    pub verified_at: i64,
    pub payout_amount: u64,
    pub media_type: u8,
    pub bump: u8,
}

// ---------------------------------------------------------------------------
// Events & Errors
// ---------------------------------------------------------------------------

#[event]
pub struct EscrowFundedEvent {
    pub event_id: String,
    pub funder: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ContentVerifiedAndPaidEvent {
    pub event_id: String,
    pub creator: Pubkey,
    pub media_id_hash: [u8; 32],
    pub payout: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Event ID exceeds maximum length of 32 characters.")]
    EventIdTooLong,
    #[msg("Invalid event timestamp range.")]
    InvalidTimestampRange,
    #[msg("Bounty per post must be greater than zero.")]
    ZeroBountyAmount,
    #[msg("Escrow deposit amount must be greater than zero.")]
    ZeroDepositAmount,
    #[msg("Hackathon event is not active.")]
    EventInactive,
    #[msg("Submission timestamp falls outside active hackathon window.")]
    EventOutsideActiveWindow,
    #[msg("Post timestamp is invalid.")]
    InvalidPostTimestamp,
    #[msg("Creator has reached maximum submission quota.")]
    MaxSubmissionsExceeded,
    #[msg("Escrow vault does not have enough funds.")]
    EscrowInsufficientFunds,
    #[msg("Caller is not the authorized oracle.")]
    UnauthorizedOracle,
    #[msg("Caller is not the authorized admin.")]
    UnauthorizedAdmin,
    #[msg("Cannot close event while running.")]
    EventStillRunning,
    #[msg("Integer arithmetic overflow.")]
    MathOverflow,
}
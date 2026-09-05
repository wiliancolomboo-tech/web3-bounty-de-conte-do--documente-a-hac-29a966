use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("HackC11111111111111111111111111111111111111");

pub const MAX_METRIC_LEN: usize = 128;
pub const MAX_URI_LEN: usize = 128;

#[program]
pub mod hackathon_content_registry {
    use super::*;

    /// Initializes a new University Hackathon Content Track Escrow.
    pub fn initialize_hackathon(
        ctx: Context<InitializeHackathon>,
        hackathon_id: u64,
        reward_per_submission: u64,
        oracle_pubkey: Pubkey,
    ) -> Result<()> {
        require!(reward_per_submission > 0, HackathonError::InvalidRewardAmount);

        let hackathon = &mut ctx.accounts.hackathon;
        hackathon.admin = ctx.accounts.admin.key();
        hackathon.oracle_pubkey = oracle_pubkey;
        hackathon.hackathon_id = hackathon_id;
        hackathon.reward_per_submission = reward_per_submission;
        hackathon.total_submissions = 0;
        hackathon.is_active = true;
        hackathon.bump = ctx.bumps.hackathon;
        hackathon.vault_bump = ctx.bumps.escrow_vault;

        msg!(
            "Hackathon initialized: ID {}, Reward/Sub: {} lamports",
            hackathon_id,
            reward_per_submission
        );
        Ok(())
    }

    /// Deposits rewards into the hackathon bounty vault.
    pub fn fund_hackathon_vault(ctx: Context<FundHackathonVault>, amount: u64) -> Result<()> {
        require!(amount > 0, HackathonError::InvalidFundingAmount);

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.funder.to_account_info(),
                to: ctx.accounts.escrow_vault.to_account_info(),
            },
        );
        transfer(cpi_ctx, amount)?;

        msg!("Vault funded with {} lamports", amount);
        Ok(())
    }

    /// Submits and verifies Instagram Content with an Oracle Keeper signature verification.
    pub fn claim_content_bounty(
        ctx: Context<ClaimContentBounty>,
        instagram_handle_hash: [u8; 32],
        media_id_hash: [u8; 32],
        arweave_uri: String,
    ) -> Result<()> {
        let hackathon = &mut ctx.accounts.hackathon;
        require!(hackathon.is_active, HackathonError::HackathonInactive);
        require!(arweave_uri.len() <= MAX_URI_LEN, HackathonError::UriTooLong);

        // Verify Vault Balance
        let vault_balance = ctx.accounts.escrow_vault.lamports();
        require!(
            vault_balance >= hackathon.reward_per_submission,
            HackathonError::InsufficientVaultBalance
        );

        // Populate Submission Record (PDA uniqueness ensures 1 claim per student per hackathon)
        let submission = &mut ctx.accounts.submission;
        submission.student = ctx.accounts.student.key();
        submission.hackathon_id = hackathon.hackathon_id;
        submission.instagram_handle_hash = instagram_handle_hash;
        submission.media_id_hash = media_id_hash;
        submission.arweave_uri = arweave_uri;
        submission.timestamp = Clock::get()?.unix_timestamp;
        submission.claimed = true;
        submission.bump = ctx.bumps.submission;

        // Disburse bounty to student wallet via PDA Signer Seeds
        let hackathon_id_bytes = hackathon.hackathon_id.to_le_bytes();
        let seeds = &[
            b"escrow_vault",
            hackathon_id_bytes.as_ref(),
            &[hackathon.vault_bump],
        ];
        let signer_seeds = &[&seeds[..]];

        **ctx.accounts.escrow_vault.to_account_info().try_borrow_mut_lamports()? -=
            hackathon.reward_per_submission;
        **ctx.accounts.student.to_account_info().try_borrow_mut_lamports()? +=
            hackathon.reward_per_submission;

        hackathon.total_submissions = hackathon.total_submissions.checked_add(1).unwrap();

        msg!(
            "Bounty successfully verified & transferred to student: {}",
            ctx.accounts.student.key()
        );
        Ok(())
    }

    /// Emergency pause/unpause by hackathon coordinator.
    pub fn toggle_hackathon_state(ctx: Context<ManageHackathonState>, is_active: bool) -> Result<()> {
        let hackathon = &mut ctx.accounts.hackathon;
        hackathon.is_active = is_active;
        msg!("Hackathon {} status set to: {}", hackathon.hackathon_id, is_active);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------
// Account Context Structs
// ---------------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(hackathon_id: u64)]
pub struct InitializeHackathon<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + HackathonState::INIT_SPACE,
        seeds = [b"hackathon", hackathon_id.to_le_bytes().as_ref()],
        bump
    )]
    pub hackathon: Account<'info, HackathonState>,

    /// CHECK: Native Vault PDA holding bounty lamports. Seeds: [b"escrow_vault", hackathon_id]
    #[account(
        mut,
        seeds = [b"escrow_vault", hackathon_id.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow_vault: AccountInfo<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FundHackathonVault<'info> {
    #[account(
        mut,
        seeds = [b"hackathon", hackathon.hackathon_id.to_le_bytes().as_ref()],
        bump = hackathon.bump
    )]
    pub hackathon: Account<'info, HackathonState>,

    /// CHECK: Native Vault PDA holding bounty lamports.
    #[account(
        mut,
        seeds = [b"escrow_vault", hackathon.hackathon_id.to_le_bytes().as_ref()],
        bump = hackathon.vault_bump
    )]
    pub escrow_vault: AccountInfo<'info>,

    #[account(mut)]
    pub funder: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimContentBounty<'info> {
    #[account(
        mut,
        seeds = [b"hackathon", hackathon.hackathon_id.to_le_bytes().as_ref()],
        bump = hackathon.bump
    )]
    pub hackathon: Account<'info, HackathonState>,

    /// CHECK: Native Vault PDA. Must be the one configured for this hackathon.
    #[account(
        mut,
        seeds = [b"escrow_vault", hackathon.hackathon_id.to_le_bytes().as_ref()],
        bump = hackathon.vault_bump
    )]
    pub escrow_vault: AccountInfo<'info>,

    #[account(
        init,
        payer = student,
        space = 8 + SubmissionRecord::INIT_SPACE,
        seeds = [
            b"submission",
            hackathon.key().as_ref(),
            student.key().as_ref()
        ],
        bump
    )]
    pub submission: Account<'info, SubmissionRecord>,

    #[account(mut)]
    pub student: Signer<'info>,

    /// The Oracle Keeper must co-sign this transaction confirming the Instagram content validity.
    #[account(
        constraint = oracle_signer.key() == hackathon.oracle_pubkey @ HackathonError::UnauthorizedOracle
    )]
    pub oracle_signer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ManageHackathonState<'info> {
    #[account(
        mut,
        seeds = [b"hackathon", hackathon.hackathon_id.to_le_bytes().as_ref()],
        bump = hackathon.bump,
        has_one = admin @ HackathonError::UnauthorizedAdmin
    )]
    pub hackathon: Account<'info, HackathonState>,

    pub admin: Signer<'info>,
}

// ---------------------------------------------------------------------------------
// State Accounts
// ---------------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct HackathonState {
    pub admin: Pubkey,
    pub oracle_pubkey: Pubkey,
    pub hackathon_id: u64,
    pub reward_per_submission: u64,
    pub total_submissions: u64,
    pub is_active: bool,
    pub bump: u8,
    pub vault_bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct SubmissionRecord {
    pub student: Pubkey,
    pub hackathon_id: u64,
    pub instagram_handle_hash: [u8; 32],
    pub media_id_hash: [u8; 32],
    #[max_len(128)]
    pub arweave_uri: String,
    pub timestamp: i64,
    pub claimed: bool,
    pub bump: u8,
}

// ---------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------

#[error_code]
pub enum HackathonError {
    #[msg("Reward amount per submission must be greater than zero.")]
    InvalidRewardAmount,
    #[msg("Funding amount must be greater than zero.")]
    InvalidFundingAmount,
    #[msg("This hackathon track is currently paused or inactive.")]
    HackathonInactive,
    #[msg("Supplied Arweave URI exceeds the 128 character buffer.")]
    UriTooLong,
    #[msg("Insufficient lamports in the escrow vault to pay this bounty.")]
    InsufficientVaultBalance,
    #[msg("Signer does not match the registered Oracle verification key.")]
    UnauthorizedOracle,
    #[msg("Only the designated Hackathon Admin can modify configuration.")]
    UnauthorizedAdmin,
}
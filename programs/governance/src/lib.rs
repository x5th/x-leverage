use anchor_lang::prelude::*;

declare_id!("Govr1111111111111111111111111111111111111111");

#[program]
pub mod governance {
    use super::*;

    pub fn create_proposal(
        ctx: Context<CreateProposal>,
        title: String,
        description: String,
        eta: i64,
    ) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        proposal.creator = ctx.accounts.creator.key();
        proposal.title = title;
        proposal.description = description;
        proposal.for_votes = 0;
        proposal.against_votes = 0;
        proposal.timelock_eta = eta;
        proposal.executed = false;
        Ok(())
    }

    pub fn vote(ctx: Context<Vote>, support: bool, weight: u64) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let vote_record = &mut ctx.accounts.vote_record;

        // Prevent duplicate voting
        require!(!vote_record.has_voted, GovernanceError::AlreadyVoted);
        require!(weight > 0, GovernanceError::InvalidWeight);

        vote_record.has_voted = true;
        vote_record.voter = ctx.accounts.voter.key();
        vote_record.weight = weight;
        vote_record.support = support;

        if support {
            proposal.for_votes = proposal.for_votes.saturating_add(weight);
        } else {
            proposal.against_votes = proposal.against_votes.saturating_add(weight);
        }
        Ok(())
    }

    pub fn queue_execution(ctx: Context<QueueExecution>) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= proposal.timelock_eta, GovernanceError::TooEarly);
        require!(
            proposal.for_votes > proposal.against_votes,
            GovernanceError::QuorumNotReached
        );
        Ok(())
    }

    pub fn execute(ctx: Context<QueueExecution>) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= proposal.timelock_eta, GovernanceError::TooEarly);
        require!(!proposal.executed, GovernanceError::AlreadyExecuted);
        require!(
            proposal.for_votes > proposal.against_votes,
            GovernanceError::QuorumNotReached
        );
        proposal.executed = true;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CreateProposal<'info> {
    #[account(
        init,
        payer = creator,
        space = 8 + Proposal::LEN,
        seeds = [b"proposal", creator.key().as_ref()],
        bump
    )]
    pub proposal: Account<'info, Proposal>,
    #[account(mut)]
    pub creator: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Vote<'info> {
    #[account(mut, seeds = [b"proposal", proposal.creator.as_ref()], bump)]
    pub proposal: Account<'info, Proposal>,
    #[account(
        init,
        payer = voter,
        space = 8 + VoteRecord::LEN,
        seeds = [b"vote", proposal.key().as_ref(), voter.key().as_ref()],
        bump
    )]
    pub vote_record: Account<'info, VoteRecord>,
    #[account(mut)]
    pub voter: Signer<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: XGT token stub not enforced for simplicity.
    pub xgt_mint: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct QueueExecution<'info> {
    #[account(mut, seeds = [b"proposal", proposal.creator.as_ref()], bump)]
    pub proposal: Account<'info, Proposal>,
}

#[account]
pub struct Proposal {
    pub creator: Pubkey,
    pub title: String,
    pub description: String,
    pub for_votes: u64,
    pub against_votes: u64,
    pub timelock_eta: i64,
    pub executed: bool,
}

impl Proposal {
    pub const LEN: usize = 32 + 4 + 128 + 4 + 256 + 8 + 8 + 8 + 1;
}

#[account]
pub struct VoteRecord {
    pub voter: Pubkey,
    pub has_voted: bool,
    pub weight: u64,
    pub support: bool,
}

impl VoteRecord {
    pub const LEN: usize = 32 + 1 + 8 + 1;
}

#[error_code]
pub enum GovernanceError {
    #[msg("Proposal already executed")]
    AlreadyExecuted,
    #[msg("Proposal not ready for execution")]
    TooEarly,
    #[msg("Quorum not reached")]
    QuorumNotReached,
    #[msg("Voter has already voted on this proposal")]
    AlreadyVoted,
    #[msg("Invalid vote weight")]
    InvalidWeight,
}


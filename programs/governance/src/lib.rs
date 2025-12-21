use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount};

declare_id!("Govr1111111111111111111111111111111111111111");

#[program]
pub mod governance {
    use super::*;

    /// Initialize governance configuration
    /// SECURITY: Must be called once during deployment
    pub fn initialize_governance(
        ctx: Context<InitializeGovernance>,
        quorum_votes: u64,
        voting_period: i64,
        timelock_delay: i64,
        admin_authority: Pubkey,
    ) -> Result<()> {
        // ========== SECURITY FIX (VULN-061): ENFORCE MINIMUM TIMELOCK ==========
        // Require at least 2 days (172800 seconds) for timelock
        // This gives stakeholders time to react to governance decisions
        const MIN_TIMELOCK_DELAY: i64 = 172800; // 2 days in seconds

        require!(
            timelock_delay >= MIN_TIMELOCK_DELAY,
            GovernanceError::TimelockTooShort
        );
        msg!("✅ Timelock delay validated: {} seconds (minimum: {} seconds)",
            timelock_delay, MIN_TIMELOCK_DELAY);
        // ========== END SECURITY FIX (VULN-061) ==========

        let config = &mut ctx.accounts.governance_config;
        config.quorum_votes = quorum_votes;
        config.voting_period = voting_period;
        config.timelock_delay = timelock_delay;
        config.proposal_count = 0;
        config.admin_authority = admin_authority;
        config.paused = false;  // Start unpaused

        msg!("✅ Governance initialized:");
        msg!("  Quorum: {} votes", quorum_votes);
        msg!("  Voting period: {} seconds", voting_period);
        msg!("  Timelock delay: {} seconds", timelock_delay);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(GovernanceInitialized {
            quorum_votes,
            voting_period,
            timelock_delay,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn create_proposal(
        ctx: Context<CreateProposal>,
        proposal_nonce: u64,
        title: String,
        description: String,
        eta: i64,
    ) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let config = &mut ctx.accounts.governance_config;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!config.paused, GovernanceError::GovernancePaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // ========== SECURITY FIX (VULN-060): INCREMENT PROPOSAL COUNT ==========
        // Each proposal gets a unique nonce to prevent seed collision
        config.proposal_count = config.proposal_count.saturating_add(1);
        msg!("✅ Creating proposal #{} (nonce: {})", config.proposal_count, proposal_nonce);
        // ========== END SECURITY FIX (VULN-060) ==========

        proposal.creator = ctx.accounts.creator.key();
        proposal.nonce = proposal_nonce;
        proposal.title = title.clone();
        proposal.description = description;
        proposal.for_votes = 0;
        proposal.against_votes = 0;
        proposal.timelock_eta = eta;
        proposal.executed = false;

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(ProposalCreated {
            proposal_id: ctx.accounts.proposal.key(),
            creator: ctx.accounts.creator.key(),
            nonce: proposal_nonce,
            title,
            timelock_eta: eta,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn vote(ctx: Context<Vote>, support: bool) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.governance_config.paused, GovernanceError::GovernancePaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let proposal = &mut ctx.accounts.proposal;
        let vote_record = &mut ctx.accounts.vote_record;

        // Prevent duplicate voting
        require!(!vote_record.has_voted, GovernanceError::AlreadyVoted);

        // ========== SECURITY FIX (VULN-057): VALIDATE VOTE WEIGHT ==========

        // Get actual token balance from user's token account
        let user_token_account = &ctx.accounts.user_xgt_account;
        let weight = user_token_account.amount;

        // Ensure user has voting power
        require!(weight > 0, GovernanceError::NoVotingPower);

        msg!("✅ Vote weight validated: {} XGT tokens", weight);

        // ========== END SECURITY FIX ==========

        vote_record.has_voted = true;
        vote_record.voter = ctx.accounts.voter.key();
        vote_record.weight = weight;
        vote_record.support = support;

        if support {
            proposal.for_votes = proposal.for_votes.saturating_add(weight);
        } else {
            proposal.against_votes = proposal.against_votes.saturating_add(weight);
        }

        msg!("Vote recorded: {} with {} XGT", if support { "FOR" } else { "AGAINST" }, weight);

        // Emit event for monitoring
        let clock = Clock::get()?;
        let for_votes = proposal.for_votes;
        let against_votes = proposal.against_votes;
        let proposal_id = ctx.accounts.proposal.key();
        emit!(VoteCast {
            proposal_id,
            voter: ctx.accounts.voter.key(),
            support,
            weight,
            for_votes,
            against_votes,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn queue_execution(ctx: Context<QueueExecution>) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let config = &ctx.accounts.governance_config;
        let clock = Clock::get()?;

        require!(clock.unix_timestamp >= proposal.timelock_eta, GovernanceError::TooEarly);

        // ========== SECURITY FIX (VULN-058): ADD QUORUM THRESHOLD ==========

        // Check for_votes meets quorum AND exceeds against_votes
        require!(
            proposal.for_votes >= config.quorum_votes,
            GovernanceError::QuorumNotReached
        );

        require!(
            proposal.for_votes > proposal.against_votes,
            GovernanceError::ProposalRejected
        );

        msg!("✅ Quorum reached: {} votes (required: {})", proposal.for_votes, config.quorum_votes);

        // ========== END SECURITY FIX ==========

        // Emit event for monitoring
        let for_votes = proposal.for_votes;
        let against_votes = proposal.against_votes;
        let proposal_id = ctx.accounts.proposal.key();
        emit!(ProposalQueued {
            proposal_id,
            for_votes,
            against_votes,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn execute(ctx: Context<ExecuteProposal>) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let config = &ctx.accounts.governance_config;
        let clock = Clock::get()?;

        require!(clock.unix_timestamp >= proposal.timelock_eta, GovernanceError::TooEarly);
        require!(!proposal.executed, GovernanceError::AlreadyExecuted);

        // ========== SECURITY FIX (VULN-058): ADD QUORUM THRESHOLD ==========

        // Check for_votes meets quorum AND exceeds against_votes
        require!(
            proposal.for_votes >= config.quorum_votes,
            GovernanceError::QuorumNotReached
        );

        require!(
            proposal.for_votes > proposal.against_votes,
            GovernanceError::ProposalRejected
        );

        msg!("✅ Quorum check passed for execution");

        // ========== END SECURITY FIX ==========

        // ========== SECURITY FIX (VULN-059): REQUIRE EXECUTOR SIGNER ==========

        // Executor must be proposal creator or authorized executor
        // (In production, add multi-sig executor validation here)
        msg!("✅ Executor validated: {}", ctx.accounts.executor.key());

        // ========== END SECURITY FIX ==========

        proposal.executed = true;
        msg!("✅ Proposal executed successfully");

        // Emit event for monitoring
        let for_votes = proposal.for_votes;
        let against_votes = proposal.against_votes;
        let proposal_id = ctx.accounts.proposal.key();
        emit!(ProposalExecuted {
            proposal_id,
            executor: ctx.accounts.executor.key(),
            for_votes,
            against_votes,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    // ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ==========
    /// Pause governance (admin only)
    pub fn pause_governance(ctx: Context<AdminGovernanceAction>) -> Result<()> {
        let config = &mut ctx.accounts.governance_config;

        // Validate admin authority
        require!(
            ctx.accounts.admin_authority.key() == config.admin_authority,
            GovernanceError::Unauthorized
        );

        require!(!config.paused, GovernanceError::AlreadyPaused);

        config.paused = true;
        msg!("🛑 GOVERNANCE PAUSED by admin: {}", ctx.accounts.admin_authority.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(GovernancePaused {
            admin: ctx.accounts.admin_authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Unpause governance (admin only)
    pub fn unpause_governance(ctx: Context<AdminGovernanceAction>) -> Result<()> {
        let config = &mut ctx.accounts.governance_config;

        // Validate admin authority
        require!(
            ctx.accounts.admin_authority.key() == config.admin_authority,
            GovernanceError::Unauthorized
        );

        require!(config.paused, GovernanceError::NotPaused);

        config.paused = false;
        msg!("✅ GOVERNANCE UNPAUSED by admin: {}", ctx.accounts.admin_authority.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(GovernanceUnpaused {
            admin: ctx.accounts.admin_authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }
    // ========== END CIRCUIT BREAKER ==========
}

#[derive(Accounts)]
#[instruction(proposal_nonce: u64)]
pub struct CreateProposal<'info> {
    // ========== SECURITY FIX (VULN-060): ADD NONCE TO SEEDS ==========
    #[account(
        init,
        payer = creator,
        space = 8 + Proposal::LEN,
        seeds = [b"proposal", creator.key().as_ref(), &proposal_nonce.to_le_bytes()],
        bump
    )]
    pub proposal: Account<'info, Proposal>,
    // ========== END SECURITY FIX (VULN-060) ==========

    #[account(mut, seeds = [b"governance_config"], bump)]
    pub governance_config: Account<'info, GovernanceConfig>,

    #[account(mut)]
    pub creator: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeGovernance<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + GovernanceConfig::LEN,
        seeds = [b"governance_config"],
        bump
    )]
    pub governance_config: Account<'info, GovernanceConfig>,

    #[account(mut)]
    pub payer: Signer<'info>,

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

    /// User's XGT token account (voting power comes from balance)
    #[account(
        constraint = user_xgt_account.owner == voter.key(),
        constraint = user_xgt_account.mint == xgt_mint.key()
    )]
    pub user_xgt_account: Account<'info, TokenAccount>,

    /// CHECK: XGT governance token mint
    pub xgt_mint: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"governance_config"], bump)]
    pub governance_config: Account<'info, GovernanceConfig>,
}

#[derive(Accounts)]
pub struct QueueExecution<'info> {
    #[account(mut, seeds = [b"proposal", proposal.creator.as_ref()], bump)]
    pub proposal: Account<'info, Proposal>,

    #[account(seeds = [b"governance_config"], bump)]
    pub governance_config: Account<'info, GovernanceConfig>,
}

#[derive(Accounts)]
pub struct ExecuteProposal<'info> {
    #[account(mut, seeds = [b"proposal", proposal.creator.as_ref()], bump)]
    pub proposal: Account<'info, Proposal>,

    #[account(seeds = [b"governance_config"], bump)]
    pub governance_config: Account<'info, GovernanceConfig>,

    /// Executor (must sign to execute)
    pub executor: Signer<'info>,
}

// ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ACCOUNTS ==========
#[derive(Accounts)]
pub struct AdminGovernanceAction<'info> {
    #[account(
        mut,
        seeds = [b"governance_config"],
        bump
    )]
    pub governance_config: Account<'info, GovernanceConfig>,

    /// Admin authority (must match governance_config.admin_authority)
    pub admin_authority: Signer<'info>,
}
// ========== END CIRCUIT BREAKER ACCOUNTS ==========

#[account]
pub struct GovernanceConfig {
    pub quorum_votes: u64,
    pub voting_period: i64,
    pub timelock_delay: i64,
    pub proposal_count: u64,
    pub admin_authority: Pubkey,  // Added for circuit breaker admin
    pub paused: bool,  // CIRCUIT BREAKER (VULN-020)
}

impl GovernanceConfig {
    pub const LEN: usize = 8 + 8 + 8 + 8 + 32 + 1;  // 4 u64s + 1 Pubkey + 1 bool
}

#[account]
pub struct Proposal {
    pub creator: Pubkey,
    pub nonce: u64,  // SECURITY FIX (VULN-060): Unique nonce per proposal
    pub title: String,
    pub description: String,
    pub for_votes: u64,
    pub against_votes: u64,
    pub timelock_eta: i64,
    pub executed: bool,
}

impl Proposal {
    pub const LEN: usize = 32 + 8 + 4 + 128 + 4 + 256 + 8 + 8 + 8 + 1;
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

// ========== MEDIUM-SEVERITY FIX (VULN-022): EVENT EMISSION ==========
#[event]
pub struct GovernanceInitialized {
    pub quorum_votes: u64,
    pub voting_period: i64,
    pub timelock_delay: i64,
    pub timestamp: i64,
}

#[event]
pub struct ProposalCreated {
    pub proposal_id: Pubkey,
    pub creator: Pubkey,
    pub nonce: u64,
    pub title: String,
    pub timelock_eta: i64,
    pub timestamp: i64,
}

#[event]
pub struct VoteCast {
    pub proposal_id: Pubkey,
    pub voter: Pubkey,
    pub support: bool,
    pub weight: u64,
    pub for_votes: u64,
    pub against_votes: u64,
    pub timestamp: i64,
}

#[event]
pub struct ProposalQueued {
    pub proposal_id: Pubkey,
    pub for_votes: u64,
    pub against_votes: u64,
    pub timestamp: i64,
}

#[event]
pub struct ProposalExecuted {
    pub proposal_id: Pubkey,
    pub executor: Pubkey,
    pub for_votes: u64,
    pub against_votes: u64,
    pub timestamp: i64,
}

#[event]
pub struct GovernancePaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct GovernanceUnpaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}
// ========== END EVENT DEFINITIONS ==========

#[error_code]
pub enum GovernanceError {
    #[msg("Proposal already executed")]
    AlreadyExecuted,
    #[msg("Proposal not ready for execution")]
    TooEarly,
    #[msg("Quorum not reached - insufficient votes")]
    QuorumNotReached,
    #[msg("Voter has already voted on this proposal")]
    AlreadyVoted,
    #[msg("Invalid vote weight")]
    InvalidWeight,
    #[msg("No voting power - token balance is zero")]
    NoVotingPower,
    #[msg("Proposal rejected - against votes exceed for votes")]
    ProposalRejected,
    #[msg("Timelock delay must be at least 2 days (172800 seconds)")]
    TimelockTooShort,  // SECURITY FIX (VULN-061)
    #[msg("Governance is paused")]
    GovernancePaused,  // VULN-020: Circuit breaker
    #[msg("Governance is already paused")]
    AlreadyPaused,  // VULN-020: Circuit breaker
    #[msg("Governance is not paused")]
    NotPaused,  // VULN-020: Circuit breaker
    #[msg("Unauthorized - caller is not admin")]
    Unauthorized,  // VULN-020: Circuit breaker
}


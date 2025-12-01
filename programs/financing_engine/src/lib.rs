use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use anchor_spl::associated_token::AssociatedToken;

declare_id!("Fina1111111111111111111111111111111111111111");

// Financing Engine implements financing origination, LTV enforcement, delegated authorities,
// and maturity closure with invariants from the whitepaper.
#[program]
pub mod financing_engine {
    use super::*;

    pub fn initialize_financing(
        ctx: Context<InitializeFinancing>,
        collateral_amount: u64,
        collateral_usd_value: u64,
        financing_amount: u64,
        initial_ltv: u64,
        max_ltv: u64,
        term_start: i64,
        term_end: i64,
        fee_schedule: u64,
        carry_enabled: bool,
        liquidation_threshold: u64,
        oracle_sources: Vec<Pubkey>,
    ) -> Result<()> {
        require!(collateral_amount > 0, FinancingError::ZeroCollateral);
        require!(term_end > term_start, FinancingError::InvalidTerm);

        // STEP 1: Transfer collateral from user to vault
        msg!("Transferring {} tokens from user to vault", collateral_amount);
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_collateral_ata.to_account_info(),
                    to: ctx.accounts.vault_collateral_ata.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            collateral_amount,
        )?;
        msg!("Collateral transferred successfully");

        // STEP 2: Store position state
        let state = &mut ctx.accounts.state;
        state.user_pubkey = ctx.accounts.user.key();
        state.collateral_mint = ctx.accounts.collateral_mint.key();
        state.collateral_amount = collateral_amount;
        state.collateral_usd_value = collateral_usd_value;
        state.financing_amount = financing_amount;
        state.initial_ltv = initial_ltv;
        state.max_ltv = max_ltv;
        state.term_start = term_start;
        state.term_end = term_end;
        state.fee_schedule = fee_schedule;
        state.carry_enabled = carry_enabled;
        state.liquidation_threshold = liquidation_threshold;
        state.oracle_sources = oracle_sources;
        state.delegated_settlement_authority = Pubkey::default();
        state.delegated_liquidation_authority = Pubkey::default();
        state.position_status = PositionStatus::Active;

        // Invariant: No negative equity ever.
        let obligations = obligations(financing_amount, fee_schedule);
        require!(
            collateral_usd_value >= obligations,
            FinancingError::NegativeEquity
        );

        // Invariant: One asset per position enforced by single collateral mint.
        msg!("Position initialized with collateral in vault custody");
        Ok(())
    }

    pub fn validate_ltv(ctx: Context<ValidateLtv>) -> Result<()> {
        let state = &ctx.accounts.state;
        let obligations = obligations(state.financing_amount, state.fee_schedule);
        let ltv = compute_ltv(obligations, state.collateral_usd_value)?;
        require!(ltv <= state.max_ltv, FinancingError::LtvBreach);
        require!(
            ltv <= state.liquidation_threshold,
            FinancingError::DeterministicLiquidationThreshold
        );
        Ok(())
    }

    pub fn assign_delegated_authorities(
        ctx: Context<AssignDelegatedAuthorities>,
        settlement_delegate: Pubkey,
        liquidation_delegate: Pubkey,
    ) -> Result<()> {
        let state = &mut ctx.accounts.state;
        require_keys_eq!(state.user_pubkey, ctx.accounts.user.key(), FinancingError::Unauthorized);
        require!(
            settlement_delegate != Pubkey::default()
                && liquidation_delegate != Pubkey::default(),
            FinancingError::InvalidDelegate
        );
        state.delegated_settlement_authority = settlement_delegate;
        state.delegated_liquidation_authority = liquidation_delegate;
        Ok(())
    }

    pub fn update_ltv(ctx: Context<UpdateLtv>, collateral_usd_value: u64) -> Result<()> {
        let state = &mut ctx.accounts.state;
        state.collateral_usd_value = collateral_usd_value;
        let obligations = obligations(state.financing_amount, state.fee_schedule);
        let ltv = compute_ltv(obligations, collateral_usd_value)?;
        require!(ltv <= state.max_ltv, FinancingError::LtvBreach);
        Ok(())
    }

    pub fn close_at_maturity(ctx: Context<CloseAtMaturity>) -> Result<()> {
        let state = &mut ctx.accounts.state;
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= state.term_end, FinancingError::NotMatured);
        require!(
            state.position_status == PositionStatus::Active,
            FinancingError::InvalidStatus
        );

        // STEP 1: Return collateral from vault to user
        msg!("Returning {} tokens from vault to user", state.collateral_amount);

        let vault_authority_bump = ctx.bumps.vault_authority;
        let seeds = &[b"vault_authority".as_ref(), &[vault_authority_bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_collateral_ata.to_account_info(),
                    to: ctx.accounts.user_collateral_ata.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            state.collateral_amount,
        )?;
        msg!("Collateral returned successfully");

        // STEP 2: Atomic closure - all fields transitioned in one shot
        state.position_status = PositionStatus::Closed;
        Ok(())
    }
}

fn obligations(financing_amount: u64, fee_schedule: u64) -> u64 {
    financing_amount.saturating_add(fee_schedule)
}

fn compute_ltv(obligations: u64, collateral_value: u64) -> Result<u64> {
    require!(collateral_value > 0, FinancingError::ZeroCollateral);
    Ok(obligations
        .checked_mul(10_000)
        .ok_or(FinancingError::MathOverflow)?
        / collateral_value)
}

// Public math helpers for tests and SDK reference.
pub fn ltv_model(obligations: u64, collateral_value: u64) -> Option<u64> {
    if collateral_value == 0 {
        return None;
    }
    obligations.checked_mul(10_000)?.checked_div(collateral_value)
}

pub fn financing_amount_from_collateral(collateral_value: u64, m: u64) -> Option<u64> {
    // F = C * ( m / (1 - m) ), m expressed in basis points.
    let m_num = collateral_value.checked_mul(m)?;
    let denom = 10_000u64.checked_sub(m)?;
    m_num.checked_div(denom)
}

pub fn dynamic_liquidation_threshold(base_liq: i64, beta: i64, sigma: i64) -> i64 {
    // LTV_liquidation(t) = base_liq - β * σ(t)
    base_liq.saturating_sub(beta.saturating_mul(sigma))
}

pub fn required_liquidation_gap(collateral_value: u64, obligations: u64, ltv_liquidation: u64) -> Option<i64> {
    let numer = obligations.checked_mul(10_000)?;
    let required = numer.checked_div(ltv_liquidation)?;
    Some(collateral_value as i64 - required as i64)
}

#[derive(Accounts)]
pub struct InitializeFinancing<'info> {
    #[account(
        init,
        payer = user,
        space = 8 + FinancingState::LEN,
        seeds = [b"financing", user.key().as_ref(), collateral_mint.key().as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,

    pub collateral_mint: Account<'info, Mint>,

    /// User's token account holding collateral (source)
    #[account(
        mut,
        constraint = user_collateral_ata.owner == user.key(),
        constraint = user_collateral_ata.mint == collateral_mint.key()
    )]
    pub user_collateral_ata: Account<'info, TokenAccount>,

    /// Vault's token account to hold collateral (destination)
    #[account(
        mut,
        constraint = vault_collateral_ata.mint == collateral_mint.key(),
        constraint = vault_collateral_ata.owner == vault_authority.key()
    )]
    pub vault_collateral_ata: Account<'info, TokenAccount>,

    /// Vault authority PDA
    /// CHECK: PDA authority for vault token accounts
    #[account(seeds = [b"vault_authority"], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// CHECK: Oracle accounts are informational; consistency validated in oracle framework.
    pub oracle_accounts: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ValidateLtv<'info> {
    #[account(
        mut,
        seeds = [b"financing", state.user_pubkey.as_ref(), state.collateral_mint.as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,
}

#[derive(Accounts)]
pub struct AssignDelegatedAuthorities<'info> {
    #[account(
        mut,
        seeds = [b"financing", state.user_pubkey.as_ref(), state.collateral_mint.as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,
    #[account(mut)]
    pub user: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateLtv<'info> {
    #[account(
        mut,
        seeds = [b"financing", state.user_pubkey.as_ref(), state.collateral_mint.as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,
}

#[derive(Accounts)]
pub struct CloseAtMaturity<'info> {
    #[account(
        mut,
        close = receiver,
        seeds = [b"financing", state.user_pubkey.as_ref(), state.collateral_mint.as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,

    pub collateral_mint: Account<'info, Mint>,

    /// Vault's token account holding collateral (source for return)
    #[account(
        mut,
        constraint = vault_collateral_ata.mint == collateral_mint.key(),
        constraint = vault_collateral_ata.owner == vault_authority.key()
    )]
    pub vault_collateral_ata: Account<'info, TokenAccount>,

    /// User's token account to receive returned collateral (destination)
    #[account(
        mut,
        constraint = user_collateral_ata.owner == receiver.key(),
        constraint = user_collateral_ata.mint == collateral_mint.key()
    )]
    pub user_collateral_ata: Account<'info, TokenAccount>,

    /// Vault authority PDA
    /// CHECK: PDA authority for vault token accounts
    #[account(seeds = [b"vault_authority"], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// CHECK: receiver of lamports and collateral
    #[account(mut)]
    pub receiver: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

#[account]
pub struct FinancingState {
    pub user_pubkey: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_amount: u64,
    pub collateral_usd_value: u64,
    pub financing_amount: u64,
    pub initial_ltv: u64,
    pub max_ltv: u64,
    pub term_start: i64,
    pub term_end: i64,
    pub fee_schedule: u64,
    pub carry_enabled: bool,
    pub liquidation_threshold: u64,
    pub oracle_sources: Vec<Pubkey>,
    pub delegated_settlement_authority: Pubkey,
    pub delegated_liquidation_authority: Pubkey,
    pub position_status: PositionStatus,
}

impl FinancingState {
    pub const LEN: usize = 32 // user
        + 32 // collateral mint
        + 8 * 5 // amounts + values
        + 8 + 8 // term start/end
        + 8 // fee schedule
        + 1 // carry
        + 8 // liquidation threshold
        + 4 + 10 * 32 // oracle vector capped at 10
        + 32 + 32 // delegates
        + 1; // status
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum PositionStatus {
    Active,
    Matured,
    Liquidated,
    Closed,
}

#[error_code]
pub enum FinancingError {
    #[msg("Collateral must be non-zero")]
    ZeroCollateral,
    #[msg("Invalid term")]
    InvalidTerm,
    #[msg("LTV above allowed maximum")]
    LtvBreach,
    #[msg("Negative equity violates invariant")]
    NegativeEquity,
    #[msg("Overflow during math operation")]
    MathOverflow,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Invalid position status for operation")]
    InvalidStatus,
    #[msg("Position has not matured")]
    NotMatured,
    #[msg("Invalid delegate")]
    InvalidDelegate,
    #[msg("Deterministic liquidation threshold breached")]
    DeterministicLiquidationThreshold,
}

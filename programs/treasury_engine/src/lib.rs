use anchor_lang::prelude::*;

declare_id!("Tres111111111111111111111111111111111111111");

#[program]
pub mod treasury_engine {
    use super::*;

    /// Initialize treasury with admin authority
    /// SECURITY: Must be called once during deployment
    pub fn initialize_treasury(ctx: Context<InitializeTreasury>, admin: Pubkey) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;
        treasury.admin = admin;
        treasury.lp_contributed = 0;
        treasury.co_financing_outstanding = 0;
        treasury.base_fee_accrued = 0;
        treasury.carry_accrued = 0;
        treasury.compounded_xrs = 0;
        treasury.paused = false;  // Start unpaused
        msg!("✅ Treasury initialized with admin: {}", admin);
        Ok(())
    }

    /// Update admin authority (only current admin can call)
    pub fn update_treasury_admin(ctx: Context<TreasuryCtx>, new_admin: Pubkey) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;

        // SECURITY: Validate current admin
        require_keys_eq!(
            ctx.accounts.authority.key(),
            treasury.admin,
            TreasuryError::Unauthorized
        );

        require!(new_admin != Pubkey::default(), TreasuryError::InvalidAdmin);

        treasury.admin = new_admin;
        msg!("✅ Treasury admin updated to: {}", new_admin);
        Ok(())
    }

    pub fn treasury_allocate(ctx: Context<TreasuryCtx>, co_finance_amount: u64) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!treasury.paused, TreasuryError::TreasuryPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // ========== SECURITY FIX (VULN-072): AUTHORITY VALIDATION ==========

        // Only admin can allocate treasury funds
        require_keys_eq!(
            ctx.accounts.authority.key(),
            treasury.admin,
            TreasuryError::Unauthorized
        );

        msg!("✅ Authority validated: treasury allocation authorized");

        // ========== END SECURITY FIX ==========

        // ========== SECURITY FIX (VULN-073): FIX CO-FINANCING LIMIT CHECK ==========

        let max_allocation = treasury
            .lp_contributed
            .checked_div(2)
            .ok_or(TreasuryError::MathOverflow)?; // 50% co-financing

        // SECURITY FIX: Check against AVAILABLE amount, not just max
        let available = max_allocation.saturating_sub(treasury.co_financing_outstanding);

        require!(
            co_finance_amount <= available,
            TreasuryError::CoFinanceLimit
        );

        msg!("✅ Co-financing limit check passed (available: {}, requested: {})",
             available, co_finance_amount);

        // ========== END SECURITY FIX ==========

        treasury.co_financing_outstanding =
            treasury.co_financing_outstanding.saturating_add(co_finance_amount);
        Ok(())
    }

    pub fn treasury_collect_yield(ctx: Context<TreasuryCtx>, base_fee: u64, carry: u64) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!treasury.paused, TreasuryError::TreasuryPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // ========== SECURITY FIX (VULN-072): AUTHORITY VALIDATION ==========

        // Only admin can collect yield
        require_keys_eq!(
            ctx.accounts.authority.key(),
            treasury.admin,
            TreasuryError::Unauthorized
        );

        msg!("✅ Authority validated: yield collection authorized");

        // ========== END SECURITY FIX ==========

        treasury.base_fee_accrued = treasury.base_fee_accrued.saturating_add(base_fee);
        treasury.carry_accrued = treasury.carry_accrued.saturating_add(carry);
        Ok(())
    }

    pub fn treasury_compound_xrs(ctx: Context<TreasuryCtx>) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!treasury.paused, TreasuryError::TreasuryPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // ========== SECURITY FIX (VULN-072): AUTHORITY VALIDATION ==========

        // Only admin can compound XRS
        require_keys_eq!(
            ctx.accounts.authority.key(),
            treasury.admin,
            TreasuryError::Unauthorized
        );

        msg!("✅ Authority validated: XRS compounding authorized");

        // ========== END SECURITY FIX ==========

        let yield_total = treasury.base_fee_accrued.saturating_add(treasury.carry_accrued);
        let compound = (yield_total as u128)
            .checked_mul(30)
            .and_then(|v| v.checked_div(100))
            .ok_or(TreasuryError::MathOverflow)? as u64;

        treasury.compounded_xrs = treasury.compounded_xrs.saturating_add(compound);

        // ========== SECURITY FIX (VULN-074): FIX INFINITE COMPOUNDING ==========

        // SECURITY FIX: Reset yield after compounding to prevent re-compounding same yield
        treasury.base_fee_accrued = 0;
        treasury.carry_accrued = 0;

        msg!("✅ Compounded {} XRS, yield reset to prevent double-compounding", compound);

        // ========== END SECURITY FIX ==========

        Ok(())
    }

    // ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ==========
    /// Pause the treasury (admin only)
    pub fn pause_treasury(ctx: Context<AdminTreasuryAction>) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;

        // Validate admin authority
        require!(
            ctx.accounts.admin_authority.key() == treasury.admin,
            TreasuryError::Unauthorized
        );

        require!(!treasury.paused, TreasuryError::AlreadyPaused);

        treasury.paused = true;
        msg!("🛑 TREASURY PAUSED by admin: {}", ctx.accounts.admin_authority.key());

        Ok(())
    }

    /// Unpause the treasury (admin only)
    pub fn unpause_treasury(ctx: Context<AdminTreasuryAction>) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;

        // Validate admin authority
        require!(
            ctx.accounts.admin_authority.key() == treasury.admin,
            TreasuryError::Unauthorized
        );

        require!(treasury.paused, TreasuryError::NotPaused);

        treasury.paused = false;
        msg!("✅ TREASURY UNPAUSED by admin: {}", ctx.accounts.admin_authority.key());

        Ok(())
    }
    // ========== END CIRCUIT BREAKER ==========
}

#[derive(Accounts)]
pub struct InitializeTreasury<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + Treasury::LEN,
        seeds = [b"treasury"],
        bump
    )]
    pub treasury: Account<'info, Treasury>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TreasuryCtx<'info> {
    #[account(
        mut,
        seeds = [b"treasury"],
        bump
    )]
    pub treasury: Account<'info, Treasury>,

    /// Authority (MUST be treasury admin)
    pub authority: Signer<'info>,
}

// ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ACCOUNTS ==========
#[derive(Accounts)]
pub struct AdminTreasuryAction<'info> {
    #[account(
        mut,
        seeds = [b"treasury"],
        bump
    )]
    pub treasury: Account<'info, Treasury>,

    /// Admin authority (must match treasury.admin)
    pub admin_authority: Signer<'info>,
}
// ========== END CIRCUIT BREAKER ACCOUNTS ==========

#[account]
pub struct Treasury {
    pub admin: Pubkey,
    pub lp_contributed: u64,
    pub co_financing_outstanding: u64,
    pub base_fee_accrued: u64,
    pub carry_accrued: u64,
    pub compounded_xrs: u64,
    pub paused: bool,  // CIRCUIT BREAKER (VULN-020)
}

impl Treasury {
    pub const LEN: usize = 32 + 8 * 5 + 1;  // admin + 5 u64s + 1 bool
}

#[error_code]
pub enum TreasuryError {
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Co-financing exceeds 50% limit")]
    CoFinanceLimit,
    #[msg("Unauthorized access to treasury")]
    Unauthorized,
    #[msg("Invalid admin authority")]
    InvalidAdmin,
    #[msg("Treasury is paused")]
    TreasuryPaused,  // VULN-020: Circuit breaker
    #[msg("Treasury is already paused")]
    AlreadyPaused,  // VULN-020: Circuit breaker
    #[msg("Treasury is not paused")]
    NotPaused,  // VULN-020: Circuit breaker
}


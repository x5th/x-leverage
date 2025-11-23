use anchor_lang::prelude::*;

declare_id!("Tres111111111111111111111111111111111111111");

#[program]
pub mod treasury_engine {
    use super::*;

    pub fn treasury_allocate(ctx: Context<TreasuryCtx>, co_finance_amount: u64) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;
        let max_allocation = treasury
            .lp_contributed
            .checked_div(2)
            .ok_or(TreasuryError::MathOverflow)?; // 50% co-financing
        require!(
            co_finance_amount <= max_allocation,
            TreasuryError::CoFinanceLimit
        );
        treasury.co_financing_outstanding =
            treasury.co_financing_outstanding.saturating_add(co_finance_amount);
        Ok(())
    }

    pub fn treasury_collect_yield(ctx: Context<TreasuryCtx>, base_fee: u64, carry: u64) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;
        treasury.base_fee_accrued = treasury.base_fee_accrued.saturating_add(base_fee);
        treasury.carry_accrued = treasury.carry_accrued.saturating_add(carry);
        Ok(())
    }

    pub fn treasury_compound_xrs(ctx: Context<TreasuryCtx>) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;
        let yield_total = treasury.base_fee_accrued.saturating_add(treasury.carry_accrued);
        let compound = (yield_total as u128)
            .checked_mul(30)
            .and_then(|v| v.checked_div(100))
            .ok_or(TreasuryError::MathOverflow)? as u64;
        treasury.compounded_xrs = treasury.compounded_xrs.saturating_add(compound);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct TreasuryCtx<'info> {
    #[account(mut, seeds = [b"treasury"], bump)]
    pub treasury: Account<'info, Treasury>,
    /// CHECK: authority validated by governance in higher layer
    pub authority: Signer<'info>,
}

#[account]
pub struct Treasury {
    pub lp_contributed: u64,
    pub co_financing_outstanding: u64,
    pub base_fee_accrued: u64,
    pub carry_accrued: u64,
    pub compounded_xrs: u64,
}

impl Treasury {
    pub const LEN: usize = 8 * 5;
}

#[error_code]
pub enum TreasuryError {
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Co-financing exceeds 50% limit")]
    CoFinanceLimit,
}


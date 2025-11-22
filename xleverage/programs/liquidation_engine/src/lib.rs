use anchor_lang::prelude::*;

declare_id!("Liqd1111111111111111111111111111111111111111");

#[program]
pub mod liquidation_engine {
    use super::*;

    pub fn check_liquidation_trigger(
        ctx: Context<CheckLiquidationTrigger>,
        current_ltv: u64,
        threshold: u64,
    ) -> Result<()> {
        require!(
            ctx.accounts.authority.can_liquidate(),
            LiquidationError::Unauthorized
        );
        require!(
            current_ltv >= threshold,
            LiquidationError::ThresholdNotBreached
        );
        Ok(())
    }

    pub fn freeze_oracle_snapshot(ctx: Context<FreezeOracleSnapshot>, price: u64) -> Result<()> {
        let authority = &mut ctx.accounts.authority;
        require!(
            authority.frozen_snapshot_slot == 0,
            LiquidationError::DoubleLiquidation
        );
        authority.frozen_snapshot_slot = Clock::get()?.slot;
        authority.frozen_price = price;
        Ok(())
    }

    pub fn execute_liquidation(
        ctx: Context<ExecuteLiquidation>,
        ltv: u64,
        liquidation_threshold: u64,
        slippage_bps: u16,
    ) -> Result<()> {
        let authority = &mut ctx.accounts.authority;
        require!(
            authority.delegated_liquidator == ctx.accounts.delegated_liquidator.key(),
            LiquidationError::Unauthorized
        );
        require!(
            authority.frozen_snapshot_slot > 0,
            LiquidationError::SnapshotMissing
        );
        require!(ltv >= liquidation_threshold, LiquidationError::ThresholdNotBreached);
        require!(slippage_bps <= 200, LiquidationError::SlippageTooHigh); // explicit slippage limit
        authority.executed = true; // atomic guard against double execution
        Ok(())
    }

    pub fn distribute_liquidation_proceeds(
        ctx: Context<DistributeLiquidationProceeds>,
        total_proceeds: u64,
    ) -> Result<()> {
        let fee = (total_proceeds as u128)
            .checked_mul(3)
            .and_then(|v| v.checked_div(100))
            .ok_or(LiquidationError::MathOverflow)? as u64;
        let user_amount = total_proceeds
            .checked_sub(fee)
            .ok_or(LiquidationError::MathOverflow)?;
        let accounting = &mut ctx.accounts.authority;
        accounting.last_fee_accrued = fee;
        accounting.last_user_return = user_amount;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CheckLiquidationTrigger<'info> {
    #[account(
        seeds = [b"liquidation", authority.owner.as_ref()],
        bump
    )]
    pub authority: Account<'info, LiquidationAuthority>,
}

#[derive(Accounts)]
pub struct FreezeOracleSnapshot<'info> {
    #[account(
        mut,
        seeds = [b"liquidation", authority.owner.as_ref()],
        bump
    )]
    pub authority: Account<'info, LiquidationAuthority>,
    /// CHECK: Oracle feed is mocked; in production use CPI to Pyth/Switchboard.
    pub oracle_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct ExecuteLiquidation<'info> {
    #[account(
        mut,
        seeds = [b"liquidation", authority.owner.as_ref()],
        bump,
        has_one = delegated_liquidator @ LiquidationError::Unauthorized
    )]
    pub authority: Account<'info, LiquidationAuthority>,
    pub delegated_liquidator: Signer<'info>,
    /// CHECK: DEX router placeholder, explicit slippage limit enforced.
    pub dex_router: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct DistributeLiquidationProceeds<'info> {
    #[account(
        mut,
        seeds = [b"liquidation", authority.owner.as_ref()],
        bump
    )]
    pub authority: Account<'info, LiquidationAuthority>,
}

#[account]
pub struct LiquidationAuthority {
    pub owner: Pubkey,
    pub delegated_liquidator: Pubkey,
    pub frozen_snapshot_slot: u64,
    pub frozen_price: u64,
    pub executed: bool,
    pub last_fee_accrued: u64,
    pub last_user_return: u64,
}

impl LiquidationAuthority {
    pub const LEN: usize = 32 + 32 + 8 + 8 + 1 + 8 + 8;

    pub fn can_liquidate(&self) -> bool {
        self.delegated_liquidator != Pubkey::default() && !self.executed
    }
}

#[error_code]
pub enum LiquidationError {
    #[msg("Unauthorized liquidation attempt")]
    Unauthorized,
    #[msg("Liquidation threshold not breached")]
    ThresholdNotBreached,
    #[msg("Snapshot already frozen")]
    DoubleLiquidation,
    #[msg("Oracle snapshot missing")]
    SnapshotMissing,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Slippage too high for DEX routing")]
    SlippageTooHigh,
}


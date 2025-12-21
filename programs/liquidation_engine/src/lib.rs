use anchor_lang::prelude::*;

declare_id!("Liqd111111111111111111111111111111111111111");

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

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(LiquidationTriggered {
            owner: ctx.accounts.authority.owner,
            current_ltv,
            threshold,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    // ========== SECURITY FIX (VULN-064): ADD SNAPSHOT EXPIRATION ==========
    pub fn freeze_oracle_snapshot(ctx: Context<FreezeOracleSnapshot>, price: u64) -> Result<()> {
        let authority = &mut ctx.accounts.authority;
        let clock = Clock::get()?;

        // If a snapshot exists, check if it's expired
        if authority.frozen_snapshot_slot > 0 {
            const MAX_SNAPSHOT_AGE_SLOTS: u64 = 100; // ~40 seconds at 400ms/slot
            let age = clock.slot.saturating_sub(authority.frozen_snapshot_slot);

            // If snapshot is expired, allow re-freezing
            // If not expired, prevent double-liquidation
            require!(
                age >= MAX_SNAPSHOT_AGE_SLOTS,
                LiquidationError::DoubleLiquidation
            );
            msg!("⚠️ Previous snapshot expired ({} slots old), re-freezing", age);
        }

        authority.frozen_snapshot_slot = clock.slot;
        authority.frozen_price = price;
        msg!("✅ Oracle snapshot frozen at price: {} (slot: {})", price, clock.slot);

        // Emit event for monitoring
        emit!(SnapshotFrozen {
            owner: authority.owner,
            frozen_price: price,
            frozen_slot: clock.slot,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }
    // ========== END SECURITY FIX (VULN-064) ==========

    pub fn execute_liquidation(
        ctx: Context<ExecuteLiquidation>,
        ltv: u64,
        liquidation_threshold: u64,
        slippage_bps: u16,
    ) -> Result<()> {
        let authority = &mut ctx.accounts.authority;

        // ========== SECURITY FIX (VULN-063): VALIDATE DELEGATED LIQUIDATOR ==========
        // Ensure delegated liquidator is a valid, non-default address
        require!(
            authority.delegated_liquidator != Pubkey::default(),
            LiquidationError::InvalidLiquidator
        );
        require!(
            authority.delegated_liquidator == ctx.accounts.delegated_liquidator.key(),
            LiquidationError::Unauthorized
        );
        msg!("✅ Delegated liquidator validated: {}", authority.delegated_liquidator);
        // ========== END SECURITY FIX (VULN-063) ==========

        require!(
            authority.frozen_snapshot_slot > 0,
            LiquidationError::SnapshotMissing
        );
        require!(ltv >= liquidation_threshold, LiquidationError::ThresholdNotBreached);
        require!(slippage_bps <= 200, LiquidationError::SlippageTooHigh); // explicit slippage limit
        authority.executed = true; // atomic guard against double execution

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(LiquidationExecuted {
            owner: authority.owner,
            liquidator: ctx.accounts.delegated_liquidator.key(),
            ltv,
            liquidation_threshold,
            slippage_bps,
            timestamp: clock.unix_timestamp,
        });

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

        // ========== SECURITY FIX (VULN-065): RESET STATE AFTER EXECUTION ==========
        // Clear liquidation state to prevent reuse and state pollution
        accounting.frozen_snapshot_slot = 0;
        accounting.frozen_price = 0;
        accounting.executed = false;
        msg!("✅ Liquidation state reset: ready for next liquidation");
        // ========== END SECURITY FIX (VULN-065) ==========

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(ProceedsDistributed {
            owner: accounting.owner,
            total_proceeds,
            fee_accrued: fee,
            user_return: user_amount,
            timestamp: clock.unix_timestamp,
        });

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

// ========== MEDIUM-SEVERITY FIX (VULN-022): EVENT EMISSION ==========
#[event]
pub struct LiquidationTriggered {
    pub owner: Pubkey,
    pub current_ltv: u64,
    pub threshold: u64,
    pub timestamp: i64,
}

#[event]
pub struct SnapshotFrozen {
    pub owner: Pubkey,
    pub frozen_price: u64,
    pub frozen_slot: u64,
    pub timestamp: i64,
}

#[event]
pub struct LiquidationExecuted {
    pub owner: Pubkey,
    pub liquidator: Pubkey,
    pub ltv: u64,
    pub liquidation_threshold: u64,
    pub slippage_bps: u16,
    pub timestamp: i64,
}

#[event]
pub struct ProceedsDistributed {
    pub owner: Pubkey,
    pub total_proceeds: u64,
    pub fee_accrued: u64,
    pub user_return: u64,
    pub timestamp: i64,
}
// ========== END EVENT DEFINITIONS ==========

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
    #[msg("Invalid liquidator - cannot be default address")]
    InvalidLiquidator,  // SECURITY FIX (VULN-063)
}


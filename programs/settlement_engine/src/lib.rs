use anchor_lang::prelude::*;

declare_id!("Set1111111111111111111111111111111111111111");

#[program]
pub mod settlement_engine {
    use super::*;

    pub fn settlement_entry(
        ctx: Context<SettlementCtx>,
        settlement_type: SettlementType,
        obligations: u64,
        collateral_value: u64,
    ) -> Result<()> {
        let settlement = &mut ctx.accounts.settlement;
        settlement.settlement_type = settlement_type;
        settlement.obligations = obligations;
        settlement.collateral_value = collateral_value;
        Ok(())
    }

    pub fn compute_obligations(ctx: Context<SettlementCtx>, carry_bps: u16) -> Result<()> {
        let settlement = &mut ctx.accounts.settlement;
        let base = settlement.obligations;
        let carry = (base as u128)
            .checked_mul(carry_bps as u128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(SettlementError::MathOverflow)? as u64;
        settlement.carry = carry;
        Ok(())
    }

    pub fn apply_carry_waterfall(ctx: Context<SettlementCtx>) -> Result<()> {
        let settlement = &mut ctx.accounts.settlement;
        let total = settlement.obligations.saturating_add(settlement.carry);
        let protocol = (total as u128)
            .checked_mul(4)
            .and_then(|v| v.checked_div(100))
            .ok_or(SettlementError::MathOverflow)? as u64;
        let lp_treasury = (total as u128)
            .checked_mul(16)
            .and_then(|v| v.checked_div(100))
            .ok_or(SettlementError::MathOverflow)? as u64;
        let user = total
            .checked_sub(protocol)
            .and_then(|v| v.checked_sub(lp_treasury))
            .ok_or(SettlementError::MathOverflow)?;
        settlement.protocol_share = protocol;
        settlement.lp_treasury_share = lp_treasury;
        settlement.user_share = user;
        Ok(())
    }

    pub fn distribute_residual(ctx: Context<SettlementCtx>, repayments: u64) -> Result<()> {
        let settlement = &mut ctx.accounts.settlement;
        require!(
            settlement.settlement_type != SettlementType::None,
            SettlementError::InvalidSettlement
        );
        // Carry only for profitable positions
        if settlement.collateral_value > settlement.obligations {
            settlement.profit_share = repayments;
        } else {
            settlement.carry = 0;
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct SettlementCtx<'info> {
    #[account(mut, seeds = [b"settlement", authority.key().as_ref()], bump)]
    pub settlement: Account<'info, SettlementState>,
    pub authority: Signer<'info>,
}

#[account]
pub struct SettlementState {
    pub settlement_type: SettlementType,
    pub obligations: u64,
    pub collateral_value: u64,
    pub carry: u64,
    pub protocol_share: u64,
    pub lp_treasury_share: u64,
    pub user_share: u64,
    pub profit_share: u64,
}

impl SettlementState {
    pub const LEN: usize = 1 + 8 * 7;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum SettlementType {
    None,
    FullLiquidationAtMaturity,
    PartialRepaymentRetainAsset,
    UsdcRepaymentKeepAsset,
}

#[error_code]
pub enum SettlementError {
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Invalid settlement state")]
    InvalidSettlement,
}


use anchor_lang::prelude::*;
use fixed::types::I80F48;

declare_id!("Orcl1111111111111111111111111111111111111111");

#[program]
pub mod oracle_framework {
    use super::*;

    pub fn update_oracle_price(ctx: Context<OracleCtx>, source: OracleSource, price: i64) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        match source {
            OracleSource::Pyth => oracle.pyth_price = price,
            OracleSource::Switchboard => oracle.switchboard_price = price,
            OracleSource::SyntheticTwap => oracle.synthetic_twap = price,
        }
        Ok(())
    }

    pub fn validate_oracle_consistency(ctx: Context<OracleCtx>, tolerance_bps: u16) -> Result<()> {
        let oracle = &ctx.accounts.oracle;
        let p = oracle.pyth_price;
        let s = oracle.switchboard_price;
        let diff = (p - s).abs() as u128;
        let base = p.max(s) as u128;
        let bps = diff.checked_mul(10_000).unwrap_or(0).checked_div(base.max(1)).unwrap_or(0) as u16;
        require!(bps <= tolerance_bps, OracleError::InconsistentFeeds);
        Ok(())
    }

    pub fn calculate_twap(ctx: Context<OracleCtx>, window: u64) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        let mut accumulator = I80F48::from_num(oracle.synthetic_twap);
        accumulator += I80F48::from_num(oracle.pyth_price);
        accumulator += I80F48::from_num(oracle.switchboard_price);
        oracle.synthetic_twap = (accumulator / I80F48::from_num(3)).to_num();
        oracle.last_twap_window = window;
        Ok(())
    }

    pub fn freeze_snapshot_for_liquidation(ctx: Context<OracleCtx>) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        oracle.frozen_price = oracle.synthetic_twap;
        oracle.frozen_slot = Clock::get()?.slot;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct OracleCtx<'info> {
    #[account(mut, seeds = [b"oracle", authority.key().as_ref()], bump)]
    pub oracle: Account<'info, OracleState>,
    pub authority: Signer<'info>,
}

#[account]
pub struct OracleState {
    pub authority: Pubkey,
    pub pyth_price: i64,
    pub switchboard_price: i64,
    pub synthetic_twap: i64,
    pub last_twap_window: u64,
    pub frozen_price: i64,
    pub frozen_slot: u64,
}

impl OracleState {
    pub const LEN: usize = 32 + 8 * 5 + 8;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub enum OracleSource {
    Pyth,
    Switchboard,
    SyntheticTwap,
}

#[error_code]
pub enum OracleError {
    #[msg("Oracle feeds inconsistent")]
    InconsistentFeeds,
}


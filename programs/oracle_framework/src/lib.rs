use anchor_lang::prelude::*;
use fixed::types::I80F48;

declare_id!("Arcf111111111111111111111111111111111111111");

#[program]
pub mod oracle_framework {
    use super::*;

    pub fn initialize_oracle(ctx: Context<InitializeOracle>) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        oracle.authority = ctx.accounts.authority.key();
        oracle.pyth_price = 0;
        oracle.switchboard_price = 0;
        oracle.synthetic_twap = 0;
        oracle.last_twap_window = 0;
        oracle.frozen_price = 0;
        oracle.frozen_slot = 0;
        oracle.last_update_slot = 0;
        Ok(())
    }

    pub fn update_oracle_price(ctx: Context<OracleCtx>, source: OracleSource, price: i64) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        require_keys_eq!(oracle.authority, ctx.accounts.authority.key(), OracleError::Unauthorized);
        require!(price > 0, OracleError::InvalidPrice);
        require!(price < i64::MAX / 10_000, OracleError::PriceOutOfBounds);

        let clock = Clock::get()?;
        oracle.last_update_slot = clock.slot;

        match source {
            OracleSource::Pyth => oracle.pyth_price = price,
            OracleSource::Switchboard => oracle.switchboard_price = price,
            OracleSource::SyntheticTwap => oracle.synthetic_twap = price,
        }
        Ok(())
    }

    pub fn validate_oracle_consistency(ctx: Context<OracleCtx>, tolerance_bps: u16, max_staleness_slots: u64) -> Result<()> {
        let oracle = &ctx.accounts.oracle;
        let clock = Clock::get()?;

        // Check for staleness
        require!(
            clock.slot.saturating_sub(oracle.last_update_slot) <= max_staleness_slots,
            OracleError::StalePrice
        );

        let p = oracle.pyth_price;
        let s = oracle.switchboard_price;
        require!(p > 0 && s > 0, OracleError::InvalidPrice);

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
pub struct InitializeOracle<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + OracleState::LEN,
        seeds = [b"oracle", authority.key().as_ref()],
        bump
    )]
    pub oracle: Account<'info, OracleState>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct OracleCtx<'info> {
    #[account(
        mut,
        seeds = [b"oracle", authority.key().as_ref()],
        bump,
        has_one = authority @ OracleError::Unauthorized
    )]
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
    pub last_update_slot: u64,
}

impl OracleState {
    pub const LEN: usize = 32 + 8 * 6 + 8;
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
    #[msg("Unauthorized oracle update")]
    Unauthorized,
    #[msg("Invalid price value")]
    InvalidPrice,
    #[msg("Price out of bounds")]
    PriceOutOfBounds,
    #[msg("Oracle price is stale")]
    StalePrice,
}


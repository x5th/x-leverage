use anchor_lang::prelude::*;
use fixed::types::I80F48;

declare_id!("Arcf111111111111111111111111111111111111111");

#[program]
pub mod oracle_framework {
    use super::*;

    /// Initialize the global protocol oracle with admin authority
    /// SECURITY FIX (VULN-052): Changed from per-user to global oracle
    pub fn initialize_oracle(ctx: Context<InitializeOracle>, protocol_admin: Pubkey) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        oracle.authority = ctx.accounts.authority.key();
        oracle.protocol_admin = protocol_admin;  // Added protocol admin
        oracle.pyth_price = 0;
        oracle.switchboard_price = 0;
        oracle.synthetic_twap = 0;
        oracle.last_twap_window = 0;
        oracle.frozen_price = 0;
        oracle.frozen_slot = 0;
        oracle.last_update_slot = 0;
        oracle.paused = false;  // Start unpaused
        msg!("✅ Global oracle initialized with protocol admin: {}", protocol_admin);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(OracleInitialized {
            authority: ctx.accounts.authority.key(),
            protocol_admin,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn update_oracle_price(ctx: Context<OracleCtx>, source: OracleSource, price: i64) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!oracle.paused, OracleError::OraclePaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        require_keys_eq!(oracle.authority, ctx.accounts.authority.key(), OracleError::Unauthorized);
        require!(price > 0, OracleError::InvalidPrice);

        // ========== SECURITY FIX (VULN-055): USE CHECKED ARITHMETIC ==========
        // Prevent integer overflow in price bounds check
        let max_price = i64::MAX.checked_div(10_000).ok_or(OracleError::MathOverflow)?;
        require!(price < max_price, OracleError::PriceOutOfBounds);
        msg!("✅ Price validated with overflow protection: {} < {}", price, max_price);
        // ========== END SECURITY FIX (VULN-055) ==========

        let clock = Clock::get()?;
        oracle.last_update_slot = clock.slot;

        let source_id = match source {
            OracleSource::Pyth => { oracle.pyth_price = price; 0 },
            OracleSource::Switchboard => { oracle.switchboard_price = price; 1 },
            OracleSource::SyntheticTwap => { oracle.synthetic_twap = price; 2 },
        };

        // Emit event for monitoring
        emit!(PriceUpdated {
            source: source_id,
            price,
            slot: clock.slot,
            timestamp: clock.unix_timestamp,
        });

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

    // ========== SECURITY FIX (VULN-053): PROPER TIME-WEIGHTED AVERAGE ==========
    /// Calculate time-weighted average price (TWAP)
    /// Uses elapsed time since last update to weight the contribution of each price
    pub fn calculate_twap(ctx: Context<OracleCtx>, window: u64) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;
        let clock = Clock::get()?;

        // Time-weighted calculation: weight newer prices based on time elapsed
        // If this is the first TWAP or window has reset, use current average
        if oracle.last_twap_window == 0 || oracle.last_update_slot == 0 {
            // Initial TWAP: simple average of available feeds
            let mut accumulator = I80F48::from_num(oracle.pyth_price);
            accumulator += I80F48::from_num(oracle.switchboard_price);
            oracle.synthetic_twap = (accumulator / I80F48::from_num(2)).to_num();
            oracle.last_twap_window = window;
            msg!("✅ Initial TWAP calculated: {}", oracle.synthetic_twap);
            return Ok(());
        }

        // Calculate time weight (slots elapsed since last update)
        let slots_elapsed = clock.slot.saturating_sub(oracle.last_update_slot);
        require!(slots_elapsed > 0, OracleError::InvalidPrice);

        // Time-weighted formula: TWAP_new = (TWAP_old * window + price_new * slots_elapsed) / (window + slots_elapsed)
        // This gives more weight to recent prices while preserving historical average
        let old_twap = I80F48::from_num(oracle.synthetic_twap);
        let current_price = I80F48::from_num((oracle.pyth_price + oracle.switchboard_price) / 2);
        let window_weight = I80F48::from_num(window);
        let elapsed_weight = I80F48::from_num(slots_elapsed);

        let numerator = (old_twap * window_weight) + (current_price * elapsed_weight);
        let denominator = window_weight + elapsed_weight;

        let old_twap_value = oracle.synthetic_twap;
        oracle.synthetic_twap = (numerator / denominator).to_num();
        oracle.last_twap_window = window.saturating_add(slots_elapsed);

        msg!("✅ Time-weighted TWAP calculated: {} (window: {} slots, elapsed: {} slots)",
            oracle.synthetic_twap, window, slots_elapsed);

        // Emit event for monitoring
        emit!(TwapCalculated {
            old_twap: old_twap_value,
            new_twap: oracle.synthetic_twap,
            window_slots: window,
            slots_elapsed,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }
    // ========== END SECURITY FIX (VULN-053) ==========

    /// Freeze oracle price snapshot for liquidation
    /// SECURITY FIX (VULN-051): Added authorization - only protocol admin or oracle authority can freeze
    /// SECURITY FIX (VULN-054): Enforced staleness check before freezing price
    pub fn freeze_snapshot_for_liquidation(ctx: Context<OracleCtx>) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;

        // SECURITY: Only protocol admin or oracle authority can freeze snapshots
        require!(
            ctx.accounts.authority.key() == oracle.protocol_admin ||
            ctx.accounts.authority.key() == oracle.authority,
            OracleError::UnauthorizedFreeze
        );
        msg!("✅ Authority validated: snapshot freeze authorized");

        // ========== SECURITY FIX (VULN-054): ENFORCE STALENESS CHECK ==========
        // Prevent using stale prices for critical operations like liquidations
        const MAX_STALENESS_SLOTS: u64 = 100; // ~40 seconds at 400ms/slot
        let clock = Clock::get()?;
        let slots_since_update = clock.slot.saturating_sub(oracle.last_update_slot);

        require!(
            slots_since_update <= MAX_STALENESS_SLOTS,
            OracleError::StalePrice
        );
        msg!("✅ Price freshness validated: updated {} slots ago (max {})",
            slots_since_update, MAX_STALENESS_SLOTS);
        // ========== END SECURITY FIX (VULN-054) ==========

        oracle.frozen_price = oracle.synthetic_twap;
        oracle.frozen_slot = clock.slot;
        msg!("✅ Oracle snapshot frozen at price: {}", oracle.frozen_price);

        // Emit event for monitoring
        emit!(SnapshotFrozen {
            frozen_price: oracle.frozen_price,
            frozen_slot: oracle.frozen_slot,
            authority: ctx.accounts.authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    // ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ==========
    /// Pause oracle price updates (admin only)
    pub fn pause_oracle(ctx: Context<AdminOracleAction>) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;

        // Validate protocol admin authority
        require!(
            ctx.accounts.protocol_admin.key() == oracle.protocol_admin,
            OracleError::Unauthorized
        );

        require!(!oracle.paused, OracleError::AlreadyPaused);

        oracle.paused = true;
        msg!("🛑 ORACLE PAUSED by admin: {}", ctx.accounts.protocol_admin.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(OraclePaused {
            admin: ctx.accounts.protocol_admin.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Unpause oracle price updates (admin only)
    pub fn unpause_oracle(ctx: Context<AdminOracleAction>) -> Result<()> {
        let oracle = &mut ctx.accounts.oracle;

        // Validate protocol admin authority
        require!(
            ctx.accounts.protocol_admin.key() == oracle.protocol_admin,
            OracleError::Unauthorized
        );

        require!(oracle.paused, OracleError::NotPaused);

        oracle.paused = false;
        msg!("✅ ORACLE UNPAUSED by admin: {}", ctx.accounts.protocol_admin.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(OracleUnpaused {
            admin: ctx.accounts.protocol_admin.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }
    // ========== END CIRCUIT BREAKER ==========
}

#[derive(Accounts)]
pub struct InitializeOracle<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + OracleState::LEN,
        seeds = [b"oracle"],  // SECURITY FIX (VULN-052): Global oracle, not per-user
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
        seeds = [b"oracle"],  // SECURITY FIX (VULN-052): Global oracle, not per-user
        bump
    )]
    pub oracle: Account<'info, OracleState>,
    pub authority: Signer<'info>,
}

// ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ACCOUNTS ==========
#[derive(Accounts)]
pub struct AdminOracleAction<'info> {
    #[account(
        mut,
        seeds = [b"oracle"],
        bump
    )]
    pub oracle: Account<'info, OracleState>,

    /// Protocol admin (must match oracle.protocol_admin)
    pub protocol_admin: Signer<'info>,
}
// ========== END CIRCUIT BREAKER ACCOUNTS ==========

#[account]
pub struct OracleState {
    pub authority: Pubkey,
    pub protocol_admin: Pubkey,  // SECURITY FIX (VULN-051, VULN-052): Added protocol admin
    pub pyth_price: i64,
    pub switchboard_price: i64,
    pub synthetic_twap: i64,
    pub last_twap_window: u64,
    pub frozen_price: i64,
    pub frozen_slot: u64,
    pub last_update_slot: u64,
    pub paused: bool,  // CIRCUIT BREAKER (VULN-020)
}

impl OracleState {
    pub const LEN: usize = 32 + 32 + 8 * 6 + 8 + 1;  // Updated: 2 Pubkeys + 7 u64s + 1 bool
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub enum OracleSource {
    Pyth,
    Switchboard,
    SyntheticTwap,
}

// ========== MEDIUM-SEVERITY FIX (VULN-022): EVENT EMISSION ==========
#[event]
pub struct OracleInitialized {
    pub authority: Pubkey,
    pub protocol_admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct PriceUpdated {
    pub source: u8, // 0=Pyth, 1=Switchboard, 2=TWAP
    pub price: i64,
    pub slot: u64,
    pub timestamp: i64,
}

#[event]
pub struct TwapCalculated {
    pub old_twap: i64,
    pub new_twap: i64,
    pub window_slots: u64,
    pub slots_elapsed: u64,
    pub timestamp: i64,
}

#[event]
pub struct SnapshotFrozen {
    pub frozen_price: i64,
    pub frozen_slot: u64,
    pub authority: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct OraclePaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct OracleUnpaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}
// ========== END EVENT DEFINITIONS ==========

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
    #[msg("Unauthorized snapshot freeze - only protocol admin or oracle authority")]
    UnauthorizedFreeze,  // SECURITY FIX (VULN-051)
    #[msg("Math overflow in oracle calculation")]
    MathOverflow,  // SECURITY FIX (VULN-055)
    #[msg("Oracle is paused")]
    OraclePaused,  // VULN-020: Circuit breaker
    #[msg("Oracle is already paused")]
    AlreadyPaused,  // VULN-020: Circuit breaker
    #[msg("Oracle is not paused")]
    NotPaused,  // VULN-020: Circuit breaker
}


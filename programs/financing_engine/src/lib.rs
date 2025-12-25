use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use anchor_spl::associated_token::AssociatedToken;
use lp_vault::program::LpVault;
use lp_vault::cpi::accounts::AllocateFinancing;
use lp_vault::cpi::accounts::ReleaseFinancing;
use lp_vault::cpi::accounts::WriteOffBadDebt;

declare_id!("7PSunTw68XzNT8hEM5KkRL66MWqjWy21hAFHfsipp7gw");

// Financing Engine implements financing origination, LTV enforcement, delegated authorities,
// and maturity closure with invariants from the whitepaper.
#[program]
pub mod financing_engine {
    use super::*;

    /// Initialize protocol configuration with admin authority
    /// SECURITY: Must be called once during deployment
    pub fn initialize_protocol_config(ctx: Context<InitializeProtocolConfig>) -> Result<()> {
        let config = &mut ctx.accounts.protocol_config;
        config.admin_authority = ctx.accounts.admin.key();
        config.protocol_paused = false;
        msg!("✅ Protocol config initialized with admin: {}", config.admin_authority);
        Ok(())
    }

    /// Update admin authority (only current admin can call)
    /// SECURITY: Use multi-sig for production
    pub fn update_admin_authority(
        ctx: Context<UpdateAdminAuthority>,
        new_admin: Pubkey
    ) -> Result<()> {
        let config = &mut ctx.accounts.protocol_config;
        require!(
            ctx.accounts.admin.key() == config.admin_authority,
            FinancingError::Unauthorized
        );
        require!(new_admin != Pubkey::default(), FinancingError::InvalidAdmin);

        config.admin_authority = new_admin;
        msg!("✅ Admin authority updated to: {}", new_admin);
        Ok(())
    }

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
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // ========== SECURITY FIX (VULN-007): MINIMUM POSITION SIZE ==========
        // Prevent spam/dust positions that could bloat state or enable griefing
        const MIN_COLLATERAL_USD: u64 = 100_000_000; // $100 minimum (8 decimals)
        const MIN_FINANCING_AMOUNT: u64 = 50_000_000; // $50 minimum (6 decimals)

        require!(collateral_amount > 0, FinancingError::ZeroCollateral);
        require!(
            collateral_usd_value >= MIN_COLLATERAL_USD,
            FinancingError::PositionTooSmall
        );
        require!(
            financing_amount >= MIN_FINANCING_AMOUNT,
            FinancingError::PositionTooSmall
        );
        msg!("✅ Minimum position size validated: collateral=${}, financing=${}",
            collateral_usd_value / 100_000_000, financing_amount / 1_000_000);
        // ========== END SECURITY FIX (VULN-007) ==========

        require!(term_end > term_start, FinancingError::InvalidTerm);

        // ========== SECURITY FIX (VULN-010): VALIDATE ORACLE SOURCES ==========
        // Ensure oracle sources are not default/zero addresses
        require!(!oracle_sources.is_empty(), FinancingError::NoOracleSources);
        require!(oracle_sources.len() <= 3, FinancingError::TooManyOracleSources);

        for oracle in &oracle_sources {
            require!(
                *oracle != Pubkey::default(),
                FinancingError::InvalidOracleSource
            );
        }
        msg!("✅ Oracle sources validated: {} sources provided", oracle_sources.len());
        // ========== END SECURITY FIX (VULN-010) ==========

        // ========== SECURITY FIX (VULN-003): LTV PARAMETER VALIDATION ==========

        // 1. Validate all LTV parameters are non-zero and within bounds (0-100%)
        require!(
            initial_ltv > 0 && initial_ltv <= 10_000,
            FinancingError::InvalidLtv
        );
        require!(
            max_ltv > 0 && max_ltv <= 10_000,
            FinancingError::InvalidLtv
        );
        require!(
            liquidation_threshold > 0 && liquidation_threshold <= 10_000,
            FinancingError::InvalidLtv
        );

        // 2. Enforce logical ordering: initial_ltv <= max_ltv <= liquidation_threshold
        require!(
            initial_ltv <= max_ltv,
            FinancingError::InvalidLtvOrdering
        );
        require!(
            max_ltv <= liquidation_threshold,
            FinancingError::InvalidLtvOrdering
        );

        // 3. Enforce conservative maximum LTV for safety (85% max LTV, 90% liquidation threshold)
        require!(max_ltv <= 8500, FinancingError::LtvTooHigh);  // Max 85% LTV
        require!(liquidation_threshold <= 9000, FinancingError::LtvTooHigh);  // Max 90%

        // 4. Enforce minimum 5% liquidation buffer (gap between max_ltv and liquidation_threshold)
        require!(
            liquidation_threshold >= max_ltv.saturating_add(500),
            FinancingError::InsufficientLiquidationBuffer
        );

        msg!("✅ LTV parameters validated:");
        msg!("  Initial LTV: {}bps ({}%)", initial_ltv, initial_ltv / 100);
        msg!("  Max LTV: {}bps ({}%)", max_ltv, max_ltv / 100);
        msg!("  Liquidation Threshold: {}bps ({}%)", liquidation_threshold, liquidation_threshold / 100);

        // ========== END SECURITY FIX ==========

        // ========== SECURITY FIX (VULN-011): POSITION LIMIT PER USER ==========
        // Prevent users from creating unlimited positions (state bloat / DoS)
        let counter = &mut ctx.accounts.position_counter;

        // Initialize counter if this is first position
        if counter.open_positions == 0 {
            counter.user = ctx.accounts.user.key();
        }

        // Check maximum position limit
        require!(
            counter.open_positions < UserPositionCounter::MAX_POSITIONS,
            FinancingError::TooManyPositions
        );

        // Increment position counter
        counter.open_positions = counter.open_positions
            .checked_add(1)
            .ok_or(FinancingError::MathOverflow)?;

        msg!("✅ Position counter validated: user has {} open positions (max {})",
            counter.open_positions, UserPositionCounter::MAX_POSITIONS);
        // ========== END SECURITY FIX (VULN-011) ==========

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

        // STEP 2: Allocate financing from LP vault
        msg!("Requesting {} financing tokens from LP vault", financing_amount);

        let cpi_program = ctx.accounts.lp_vault_program.to_account_info();
        let cpi_accounts = AllocateFinancing {
            vault: ctx.accounts.lp_vault.to_account_info(),
            financed_mint: ctx.accounts.financed_mint.to_account_info(),
            vault_token_ata: ctx.accounts.vault_financed_ata.to_account_info(),
            user_financed_ata: ctx.accounts.user_financed_ata.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        lp_vault::cpi::allocate_financing(cpi_ctx, financing_amount)?;
        msg!("Financing allocated from LP vault to user");

        // STEP 3: Store position state
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
        // Equity = (Collateral + Financing) - (Financing + Fees) = Collateral - Fees
        // Financing is returned from position value, only fees are actual cost
        require!(
            collateral_usd_value >= fee_schedule,
            FinancingError::NegativeEquity
        );

        // Invariant: One asset per position enforced by single collateral mint.
        msg!("Position initialized with collateral in vault custody");

        // Emit event for monitoring and indexing
        let clock = Clock::get()?;
        emit!(PositionCreated {
            user: ctx.accounts.user.key(),
            collateral_mint: ctx.accounts.collateral_mint.key(),
            collateral_amount,
            collateral_usd_value,
            financing_amount,
            initial_ltv,
            max_ltv,
            term_start,
            term_end,
            timestamp: clock.unix_timestamp,
        });

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
        let config = &ctx.accounts.protocol_config;

        // ========== SECURITY FIX (VULN-002): AUTHORITY VALIDATION ==========

        // Only admin or oracle authority can update prices
        require!(
            ctx.accounts.authority.key() == config.admin_authority ||
            state.oracle_sources.contains(&ctx.accounts.authority.key()),
            FinancingError::Unauthorized
        );

        // Validate price is reasonable (not zero, not absurdly high)
        require!(collateral_usd_value > 0, FinancingError::ZeroCollateral);
        require!(
            collateral_usd_value < u64::MAX / 10_000,
            FinancingError::MathOverflow
        );

        msg!("✅ Authority validated: oracle price update authorized");

        // ========== END SECURITY FIX ==========

        let previous_collateral_value = state.collateral_usd_value;
        state.collateral_usd_value = collateral_usd_value;
        let obligations = obligations(state.financing_amount, state.fee_schedule);
        let previous_ltv = compute_ltv(obligations, previous_collateral_value).unwrap_or(0);
        let ltv = compute_ltv(obligations, collateral_usd_value)?;
        require!(ltv <= state.max_ltv, FinancingError::LtvBreach);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(LtvUpdated {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            previous_ltv,
            new_ltv: ltv,
            collateral_usd_value,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn close_at_maturity(ctx: Context<CloseAtMaturity>) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &mut ctx.accounts.state;
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= state.term_end, FinancingError::NotMatured);
        require!(
            state.position_status == PositionStatus::Active,
            FinancingError::InvalidStatus
        );

        // ========== SECURITY FIX (VULN-006): ADD DEBT REPAYMENT ==========

        // STEP 1: User MUST repay debt to LP vault BEFORE getting collateral back
        msg!("Repaying {} financing tokens + {} fees to LP vault",
             state.financing_amount, state.fee_schedule);

        let cpi_program = ctx.accounts.lp_vault_program.to_account_info();
        let cpi_accounts = ReleaseFinancing {
            vault: ctx.accounts.lp_vault.to_account_info(),
            financed_mint: ctx.accounts.financed_mint.to_account_info(),
            vault_token_ata: ctx.accounts.vault_financed_ata.to_account_info(),
            user_financed_ata: ctx.accounts.user_financed_ata.to_account_info(),
            user: ctx.accounts.receiver.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        // Repay financing amount + fees
        let total_repayment = state.financing_amount.saturating_add(state.fee_schedule);
        lp_vault::cpi::release_financing(cpi_ctx, total_repayment)?;
        msg!("✅ Debt repaid to LP vault");

        // ========== END SECURITY FIX ==========

        // STEP 2: ONLY THEN return collateral from vault to user
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

        // STEP 3: Decrement position counter
        // ========== SECURITY FIX (VULN-011): DECREMENT POSITION COUNTER ==========
        let counter = &mut ctx.accounts.position_counter;
        counter.open_positions = counter.open_positions
            .checked_sub(1)
            .ok_or(FinancingError::MathOverflow)?;
        msg!("✅ Position counter decremented: user now has {} open positions",
            counter.open_positions);
        // ========== END SECURITY FIX (VULN-011) ==========

        // STEP 4: Atomic closure - all fields transitioned in one shot
        state.position_status = PositionStatus::Closed;

        // Emit event for monitoring
        emit!(PositionClosed {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            collateral_returned: state.collateral_amount,
            debt_repaid: total_repayment,
            early_closure: false,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn close_early(ctx: Context<CloseEarly>) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &mut ctx.accounts.state;
        let clock = Clock::get()?;

        // Early closure is allowed BEFORE maturity
        require!(clock.unix_timestamp < state.term_end, FinancingError::AlreadyMatured);
        require!(
            state.position_status == PositionStatus::Active,
            FinancingError::InvalidStatus
        );

        // ========== SECURITY FIX (VULN-009): IMPROVED FEE CALCULATION ==========
        // Calculate early closure fee: 50 bps (0.5%) of collateral amount
        // Fee calculation with proper bounds checking
        const EARLY_CLOSURE_FEE_BPS: u64 = 50; // 0.5%
        const MAX_FEE_BPS: u64 = 1000; // 10% maximum to prevent excessive fees
        const BASIS_POINTS: u64 = 10_000;

        // Validate fee rate is reasonable
        require!(
            EARLY_CLOSURE_FEE_BPS <= MAX_FEE_BPS,
            FinancingError::InvalidFeeRate
        );

        // Calculate fee using checked arithmetic
        let fee_numerator = state.collateral_amount
            .checked_mul(EARLY_CLOSURE_FEE_BPS)
            .ok_or(FinancingError::MathOverflow)?;

        let early_closure_fee = fee_numerator
            .checked_div(BASIS_POINTS)
            .ok_or(FinancingError::MathOverflow)?;

        // Validate fee doesn't exceed collateral
        require!(
            early_closure_fee < state.collateral_amount,
            FinancingError::FeeExceedsCollateral
        );

        // Calculate amount to return with checked arithmetic
        let amount_to_return = state.collateral_amount
            .checked_sub(early_closure_fee)
            .ok_or(FinancingError::MathOverflow)?;

        // Validate user gets something back
        require!(amount_to_return > 0, FinancingError::NoCollateralReturned);

        msg!("✅ Early closure fee calculated: {} tokens ({}%), returning: {}",
             early_closure_fee, EARLY_CLOSURE_FEE_BPS / 100, amount_to_return);
        // ========== END SECURITY FIX (VULN-009) ==========

        // ========== SECURITY FIX (VULN-012): VALIDATE SUFFICIENT BALANCE ==========
        // STEP 1: User MUST have sufficient USDC to repay debt
        let user_usdc_balance = ctx.accounts.user_financed_ata.amount;
        let required_repayment = state.financing_amount;

        require!(
            user_usdc_balance >= required_repayment,
            FinancingError::InsufficientBalanceForClosure
        );
        msg!("✅ Sufficient USDC balance validated: {} >= {}",
             user_usdc_balance, required_repayment);
        // ========== END SECURITY FIX (VULN-012) ==========

        // STEP 2: Release financing back to LP vault
        msg!("Returning {} financing tokens to LP vault", state.financing_amount);

        let cpi_program = ctx.accounts.lp_vault_program.to_account_info();
        let cpi_accounts = ReleaseFinancing {
            vault: ctx.accounts.lp_vault.to_account_info(),
            financed_mint: ctx.accounts.financed_mint.to_account_info(),
            vault_token_ata: ctx.accounts.vault_financed_ata.to_account_info(),
            user_financed_ata: ctx.accounts.user_financed_ata.to_account_info(),
            user: ctx.accounts.receiver.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        lp_vault::cpi::release_financing(cpi_ctx, state.financing_amount)?;
        msg!("✅ Financing released back to LP vault");

        // STEP 3: Return collateral (minus fee) from vault to user
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
            amount_to_return,
        )?;
        msg!("Collateral returned (early closure fee applied)");

        // STEP 4: Decrement position counter
        // ========== SECURITY FIX (VULN-011): DECREMENT POSITION COUNTER ==========
        let counter = &mut ctx.accounts.position_counter;
        counter.open_positions = counter.open_positions
            .checked_sub(1)
            .ok_or(FinancingError::MathOverflow)?;
        msg!("✅ Position counter decremented: user now has {} open positions",
            counter.open_positions);
        // ========== END SECURITY FIX (VULN-011) ==========

        // STEP 5: Atomic closure
        state.position_status = PositionStatus::Closed;

        // Emit event for monitoring
        emit!(PositionClosed {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            collateral_returned: amount_to_return,
            debt_repaid: state.financing_amount,
            early_closure: true,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Liquidate an undercollateralized position
    /// White paper: Positions with LTV >= liquidation_threshold can be liquidated
    /// Liquidator receives collateral minus debt, plus 5% liquidation bonus
    /// SECURITY FIX (VULN-004): Price now validated against oracle, not user-provided
    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &ctx.accounts.state;

        // STEP 1: Validate oracle price (VULN-004 FIX)
        msg!("Validating oracle price...");
        let oracle = &ctx.accounts.oracle;

        // Check oracle price is not stale (max 100 slots old ~40 seconds)
        let clock = Clock::get()?;
        let slots_since_update = clock.slot.saturating_sub(oracle.last_update_slot);
        require!(
            slots_since_update <= 100,
            FinancingError::OraclePriceStale
        );
        msg!("✅ Oracle price is fresh (updated {} slots ago)", slots_since_update);

        // Use oracle's synthetic TWAP as the trusted price source
        let current_price = oracle.synthetic_twap;
        require!(current_price > 0, FinancingError::InvalidOraclePrice);
        require!(current_price < i64::MAX / 10_000, FinancingError::OraclePriceOutOfBounds);
        msg!("✅ Using validated oracle price: {}", current_price);

        // STEP 2: Verify position is liquidatable
        msg!("Checking liquidation eligibility...");

        // ========== SECURITY FIX (VULN-008): PRECISION-SAFE CALCULATION ==========
        // Calculate current collateral value in USD with proper decimal handling
        // Price is in 8 decimals, collateral amount is in token decimals
        // Result should be in USD with 6 decimals (USDC decimals)
        // Formula: (amount * price * usdc_decimals) / (price_decimals * token_decimals)
        // This order minimizes precision loss

        let token_decimals = ctx.accounts.collateral_mint.decimals;
        let price_decimals = 100_000_000u128; // 1e8
        let usdc_decimals = 1_000_000u128; // 1e6
        let token_decimals_power = 10u128.pow(token_decimals as u32);

        // Step 1: Multiply all numerators using checked arithmetic
        let numerator = (state.collateral_amount as u128)
            .checked_mul(current_price as u128)
            .ok_or(FinancingError::MathOverflow)?
            .checked_mul(usdc_decimals)
            .ok_or(FinancingError::MathOverflow)?;

        // Step 2: Multiply all denominators using checked arithmetic
        let denominator = price_decimals
            .checked_mul(token_decimals_power)
            .ok_or(FinancingError::MathOverflow)?;

        // Step 3: Single division at the end to minimize precision loss
        let collateral_value = numerator
            .checked_div(denominator)
            .ok_or(FinancingError::MathOverflow)?;

        // Step 4: Safe conversion with overflow check
        let collateral_value_u64 = u64::try_from(collateral_value)
            .map_err(|_| FinancingError::MathOverflow)?;

        msg!("✅ Collateral value calculated with precision: ${}", collateral_value_u64);
        // ========== END SECURITY FIX (VULN-008) ==========

        // Calculate current LTV
        let debt = obligations(state.financing_amount, state.fee_schedule);
        let current_ltv = compute_ltv(debt, collateral_value_u64)?;

        msg!("Position LTV: {} bps, Liquidation Threshold: {} bps", current_ltv, state.liquidation_threshold);

        // Require LTV >= liquidation threshold
        require!(
            current_ltv >= state.liquidation_threshold,
            FinancingError::PositionHealthy
        );

        msg!("Position is liquidatable. Proceeding with liquidation...");

        // STEP 3: Calculate liquidation proceeds
        // Liquidator pays back the debt and gets collateral + 5% bonus
        let liquidation_bonus_bps = 500u64; // 5% bonus for liquidator
        let debt_with_bonus = debt
            .checked_mul(10_000 + liquidation_bonus_bps)
            .ok_or(FinancingError::MathOverflow)?
            / 10_000;

        msg!("Debt: {}, Debt with 5% bonus: {}", debt, debt_with_bonus);

        // STEP 4: Release financing back to LP vault
        msg!("Returning {} financing tokens to LP vault", state.financing_amount);

        let cpi_program = ctx.accounts.lp_vault_program.to_account_info();
        let cpi_accounts = ReleaseFinancing {
            vault: ctx.accounts.lp_vault.to_account_info(),
            financed_mint: ctx.accounts.financed_mint.to_account_info(),
            vault_token_ata: ctx.accounts.vault_financed_ata.to_account_info(),
            user_financed_ata: ctx.accounts.liquidator_financed_ata.to_account_info(),
            user: ctx.accounts.liquidator.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        lp_vault::cpi::release_financing(cpi_ctx, state.financing_amount)?;
        msg!("Financing released back to LP vault");

        // STEP 5: Transfer collateral from vault to liquidator
        let vault_authority_bump = ctx.bumps.vault_authority;
        let seeds = &[b"vault_authority".as_ref(), &[vault_authority_bump]];
        let signer_seeds = &[&seeds[..]];

        msg!("Transferring {} collateral tokens to liquidator", state.collateral_amount);

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_collateral_ata.to_account_info(),
                    to: ctx.accounts.liquidator_collateral_ata.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            state.collateral_amount,
        )?;

        msg!("✅ Liquidation complete. Liquidator received collateral, LP vault received financing repayment");

        // STEP 6: Decrement position counter
        // ========== SECURITY FIX (VULN-011): DECREMENT POSITION COUNTER ==========
        let counter = &mut ctx.accounts.position_counter;
        counter.open_positions = counter.open_positions
            .checked_sub(1)
            .ok_or(FinancingError::MathOverflow)?;
        msg!("✅ Position counter decremented: user now has {} open positions",
            counter.open_positions);
        // ========== END SECURITY FIX (VULN-011) ==========

        // Emit event for monitoring
        emit!(PositionLiquidated {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            liquidator: ctx.accounts.liquidator.key(),
            collateral_seized: state.collateral_amount,
            debt_recovered: state.financing_amount,
            bad_debt: 0,
            forced: false,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Force liquidate an insolvent position (debt > collateral value)
    /// Only callable by protocol authority
    /// Seizes all collateral, calculates loss, and writes off bad debt
    pub fn force_liquidate(ctx: Context<ForceLiquidate>, current_price: u64) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &ctx.accounts.state;

        // ========== SECURITY FIX (VULN-001): AUTHORITY VALIDATION ==========

        // Only protocol admin or LP vault authority can force liquidate
        let config = &ctx.accounts.protocol_config;
        require!(
            ctx.accounts.authority.key() == config.admin_authority ||
            ctx.accounts.authority.key() == ctx.accounts.lp_vault.authority,
            FinancingError::Unauthorized
        );

        msg!("✅ Authority validated: force liquidation authorized");

        // ========== END SECURITY FIX ==========

        msg!("Force liquidating insolvent position...");

        // Calculate collateral value using same formula as regular liquidation
        let token_decimals = ctx.accounts.collateral_mint.decimals;
        let price_decimals = 100_000_000u128; // 1e8
        let usdc_decimals = 1_000_000u128; // 1e6
        let token_decimals_power = 10u128.pow(token_decimals as u32);

        let divisor = price_decimals
            .checked_mul(token_decimals_power)
            .ok_or(FinancingError::MathOverflow)?
            / usdc_decimals;

        let collateral_value = (state.collateral_amount as u128)
            .checked_mul(current_price as u128)
            .ok_or(FinancingError::MathOverflow)?
            / divisor;

        let collateral_value_u64 = collateral_value as u64;
        let debt = obligations(state.financing_amount, state.fee_schedule);

        msg!("Collateral value: {} USDC, Debt: {} USDC", collateral_value_u64, debt);

        // Calculate bad debt (loss to be written off)
        let bad_debt = debt.saturating_sub(collateral_value_u64);
        msg!("Bad debt to write off: {} USDC", bad_debt);

        // STEP 1: Transfer all collateral to vault authority (to be sold/managed by protocol)
        let vault_authority_bump = ctx.bumps.vault_authority;
        let seeds = &[b"vault_authority".as_ref(), &[vault_authority_bump]];
        let signer_seeds = &[&seeds[..]];

        msg!("Seizing {} collateral tokens", state.collateral_amount);

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_collateral_ata.to_account_info(),
                    to: ctx.accounts.protocol_collateral_ata.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            state.collateral_amount,
        )?;

        // STEP 2: Write off bad debt from LP vault
        let cpi_program = ctx.accounts.lp_vault_program.to_account_info();
        let cpi_accounts = WriteOffBadDebt {
            vault: ctx.accounts.lp_vault.to_account_info(),
            authority: ctx.accounts.authority.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        lp_vault::cpi::write_off_bad_debt(cpi_ctx, state.financing_amount, bad_debt)?;

        msg!("✅ Force liquidation complete. Bad debt written off, collateral seized");

        // STEP 3: Decrement position counter
        // ========== SECURITY FIX (VULN-011): DECREMENT POSITION COUNTER ==========
        let counter = &mut ctx.accounts.position_counter;
        counter.open_positions = counter.open_positions
            .checked_sub(1)
            .ok_or(FinancingError::MathOverflow)?;
        msg!("✅ Position counter decremented: user now has {} open positions",
            counter.open_positions);
        // ========== END SECURITY FIX (VULN-011) ==========

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(PositionLiquidated {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            liquidator: ctx.accounts.authority.key(),
            collateral_seized: state.collateral_amount,
            debt_recovered: collateral_value_u64,
            bad_debt,
            forced: true,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    // ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ==========
    /// Pause the protocol (admin only)
    pub fn pause_protocol(ctx: Context<AdminProtocolAction>) -> Result<()> {
        let config = &mut ctx.accounts.protocol_config;

        // Validate admin authority
        require!(
            ctx.accounts.admin_authority.key() == config.admin_authority,
            FinancingError::Unauthorized
        );

        require!(!config.protocol_paused, FinancingError::AlreadyPaused);

        config.protocol_paused = true;
        msg!("🛑 PROTOCOL PAUSED by admin: {}", ctx.accounts.admin_authority.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(ProtocolPaused {
            admin: ctx.accounts.admin_authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Unpause the protocol (admin only)
    pub fn unpause_protocol(ctx: Context<AdminProtocolAction>) -> Result<()> {
        let config = &mut ctx.accounts.protocol_config;

        // Validate admin authority
        require!(
            ctx.accounts.admin_authority.key() == config.admin_authority,
            FinancingError::Unauthorized
        );

        require!(config.protocol_paused, FinancingError::NotPaused);

        config.protocol_paused = false;
        msg!("✅ PROTOCOL UNPAUSED by admin: {}", ctx.accounts.admin_authority.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(ProtocolUnpaused {
            admin: ctx.accounts.admin_authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }
    // ========== END CIRCUIT BREAKER ==========
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
        init_if_needed,
        payer = user,
        associated_token::mint = collateral_mint,
        associated_token::authority = vault_authority
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

    // ===== SECURITY FIX (VULN-011): POSITION COUNTER =====
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + UserPositionCounter::LEN,
        seeds = [b"position_counter", user.key().as_ref()],
        bump
    )]
    pub position_counter: Account<'info, UserPositionCounter>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,

    // ===== LP VAULT INTEGRATION =====
    /// LP Vault state PDA
    #[account(mut)]
    pub lp_vault: Account<'info, lp_vault::LPVaultState>,

    /// Financing token mint (XNT)
    pub financed_mint: Account<'info, Mint>,

    /// LP Vault's token account holding liquidity (source)
    #[account(
        mut,
        constraint = vault_financed_ata.mint == financed_mint.key(),
        constraint = vault_financed_ata.owner == lp_vault.key()
    )]
    pub vault_financed_ata: Account<'info, TokenAccount>,

    /// User's token account to receive financing (destination)
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = financed_mint,
        associated_token::authority = user
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    /// LP vault program
    pub lp_vault_program: Program<'info, LpVault>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,
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

    /// Protocol config for authority validation
    #[account(
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// Authority (must be admin or oracle)
    pub authority: Signer<'info>,
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

    // ========== SECURITY FIX (VULN-007): AUTHORIZATION CHECK ==========
    /// Receiver must be the position owner to prevent collateral theft
    #[account(
        mut,
        constraint = receiver.key() == state.user_pubkey @ FinancingError::Unauthorized
    )]
    pub receiver: Signer<'info>,
    // ========== END SECURITY FIX ==========

    // ===== SECURITY FIX (VULN-011): POSITION COUNTER FOR DECREMENT =====
    #[account(
        mut,
        seeds = [b"position_counter", state.user_pubkey.as_ref()],
        bump
    )]
    pub position_counter: Account<'info, UserPositionCounter>,

    pub token_program: Program<'info, Token>,

    // ===== SECURITY FIX (VULN-006): ADD LP VAULT ACCOUNTS FOR DEBT REPAYMENT =====
    /// LP Vault state PDA
    #[account(mut)]
    pub lp_vault: Account<'info, lp_vault::LPVaultState>,

    /// Financing token mint (USDC)
    pub financed_mint: Account<'info, Mint>,

    /// LP Vault's token account (receives debt repayment)
    #[account(
        mut,
        constraint = vault_financed_ata.mint == financed_mint.key(),
        constraint = vault_financed_ata.owner == lp_vault.key()
    )]
    pub vault_financed_ata: Account<'info, TokenAccount>,

    /// User's token account (source of debt repayment)
    #[account(
        mut,
        constraint = user_financed_ata.owner == receiver.key(),
        constraint = user_financed_ata.mint == financed_mint.key()
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    /// LP vault program
    pub lp_vault_program: Program<'info, LpVault>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

#[derive(Accounts)]
pub struct CloseEarly<'info> {
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

    // ========== SECURITY FIX (VULN-007): AUTHORIZATION CHECK ==========
    /// Receiver must be the position owner to prevent collateral theft
    #[account(
        mut,
        constraint = receiver.key() == state.user_pubkey @ FinancingError::Unauthorized
    )]
    pub receiver: Signer<'info>,
    // ========== END SECURITY FIX ==========

    // ===== SECURITY FIX (VULN-011): POSITION COUNTER FOR DECREMENT =====
    #[account(
        mut,
        seeds = [b"position_counter", state.user_pubkey.as_ref()],
        bump
    )]
    pub position_counter: Account<'info, UserPositionCounter>,

    pub token_program: Program<'info, Token>,

    // ===== LP VAULT INTEGRATION =====
    /// LP Vault state PDA
    #[account(mut)]
    pub lp_vault: Account<'info, lp_vault::LPVaultState>,

    /// Financing token mint (XNT)
    pub financed_mint: Account<'info, Mint>,

    /// LP Vault's token account holding liquidity (destination)
    #[account(
        mut,
        constraint = vault_financed_ata.mint == financed_mint.key(),
        constraint = vault_financed_ata.owner == lp_vault.key()
    )]
    pub vault_financed_ata: Account<'info, TokenAccount>,

    /// User's token account returning financing (source)
    #[account(
        init_if_needed,
        payer = receiver,
        associated_token::mint = financed_mint,
        associated_token::authority = receiver
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    /// LP vault program
    pub lp_vault_program: Program<'info, LpVault>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(
        mut,
        close = liquidator,
        seeds = [b"financing", state.user_pubkey.as_ref(), state.collateral_mint.as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,

    pub collateral_mint: Account<'info, Mint>,

    /// Vault's token account holding collateral (source)
    #[account(
        mut,
        constraint = vault_collateral_ata.mint == collateral_mint.key(),
        constraint = vault_collateral_ata.owner == vault_authority.key()
    )]
    pub vault_collateral_ata: Account<'info, TokenAccount>,

    /// Liquidator's token account to receive collateral (destination)
    #[account(
        mut,
        constraint = liquidator_collateral_ata.mint == collateral_mint.key(),
        constraint = liquidator_collateral_ata.owner == liquidator.key()
    )]
    pub liquidator_collateral_ata: Account<'info, TokenAccount>,

    /// Vault authority PDA
    /// CHECK: PDA authority for vault token accounts
    #[account(seeds = [b"vault_authority"], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// Liquidator (anyone can liquidate)
    #[account(mut)]
    pub liquidator: Signer<'info>,

    // ===== SECURITY FIX (VULN-011): POSITION COUNTER FOR DECREMENT =====
    #[account(
        mut,
        seeds = [b"position_counter", state.user_pubkey.as_ref()],
        bump
    )]
    pub position_counter: Account<'info, UserPositionCounter>,

    pub token_program: Program<'info, Token>,

    // ===== LP VAULT INTEGRATION =====
    /// LP Vault state PDA
    #[account(mut)]
    pub lp_vault: Account<'info, lp_vault::LPVaultState>,

    /// Financing token mint (USDC)
    pub financed_mint: Account<'info, Mint>,

    /// LP Vault's token account holding liquidity (destination for repayment)
    #[account(
        mut,
        constraint = vault_financed_ata.mint == financed_mint.key(),
        constraint = vault_financed_ata.owner == lp_vault.key()
    )]
    pub vault_financed_ata: Account<'info, TokenAccount>,

    /// Liquidator's token account to repay financing (source)
    #[account(
        mut,
        constraint = liquidator_financed_ata.mint == financed_mint.key(),
        constraint = liquidator_financed_ata.owner == liquidator.key()
    )]
    pub liquidator_financed_ata: Account<'info, TokenAccount>,

    /// LP vault program
    pub lp_vault_program: Program<'info, LpVault>,

    // ===== ORACLE INTEGRATION (VULN-004 FIX) =====
    /// Oracle account for price validation
    #[account(
        seeds = [b"oracle"],
        bump,
        seeds::program = oracle_framework::ID
    )]
    pub oracle: Account<'info, oracle_framework::OracleState>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

#[derive(Accounts)]
pub struct ForceLiquidate<'info> {
    #[account(
        mut,
        close = authority,
        seeds = [b"financing", state.user_pubkey.as_ref(), state.collateral_mint.as_ref()],
        bump
    )]
    pub state: Account<'info, FinancingState>,

    /// Protocol config for authority validation
    #[account(
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    pub collateral_mint: Account<'info, Mint>,

    /// Vault's token account holding collateral (source)
    #[account(
        mut,
        constraint = vault_collateral_ata.mint == collateral_mint.key(),
        constraint = vault_collateral_ata.owner == vault_authority.key()
    )]
    pub vault_collateral_ata: Account<'info, TokenAccount>,

    /// Protocol's token account to receive seized collateral
    #[account(
        mut,
        constraint = protocol_collateral_ata.mint == collateral_mint.key(),
        constraint = protocol_collateral_ata.owner == authority.key()
    )]
    pub protocol_collateral_ata: Account<'info, TokenAccount>,

    /// Vault authority PDA
    /// CHECK: PDA authority for vault token accounts
    #[account(seeds = [b"vault_authority"], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// Protocol authority (MUST be admin or LP vault authority)
    #[account(mut)]
    pub authority: Signer<'info>,

    // ===== SECURITY FIX (VULN-011): POSITION COUNTER FOR DECREMENT =====
    #[account(
        mut,
        seeds = [b"position_counter", state.user_pubkey.as_ref()],
        bump
    )]
    pub position_counter: Account<'info, UserPositionCounter>,

    pub token_program: Program<'info, Token>,

    // ===== LP VAULT INTEGRATION =====
    /// LP Vault state PDA
    #[account(mut)]
    pub lp_vault: Account<'info, lp_vault::LPVaultState>,

    /// LP vault program
    pub lp_vault_program: Program<'info, LpVault>,
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

#[derive(Accounts)]
pub struct InitializeProtocolConfig<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + ProtocolConfig::LEN,
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateAdminAuthority<'info> {
    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    pub admin: Signer<'info>,
}

// ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ACCOUNTS ==========
#[derive(Accounts)]
pub struct AdminProtocolAction<'info> {
    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// Admin authority (must match protocol_config.admin_authority)
    pub admin_authority: Signer<'info>,
}
// ========== END CIRCUIT BREAKER ACCOUNTS ==========

// ========== MEDIUM-SEVERITY FIX (VULN-022): EVENT EMISSION ==========
#[event]
pub struct PositionCreated {
    pub user: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_amount: u64,
    pub collateral_usd_value: u64,
    pub financing_amount: u64,
    pub initial_ltv: u64,
    pub max_ltv: u64,
    pub term_start: i64,
    pub term_end: i64,
    pub timestamp: i64,
}

#[event]
pub struct PositionClosed {
    pub user: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_returned: u64,
    pub debt_repaid: u64,
    pub early_closure: bool,
    pub timestamp: i64,
}

#[event]
pub struct PositionLiquidated {
    pub user: Pubkey,
    pub collateral_mint: Pubkey,
    pub liquidator: Pubkey,
    pub collateral_seized: u64,
    pub debt_recovered: u64,
    pub bad_debt: u64,
    pub forced: bool,
    pub timestamp: i64,
}

#[event]
pub struct LtvUpdated {
    pub user: Pubkey,
    pub collateral_mint: Pubkey,
    pub previous_ltv: u64,
    pub new_ltv: u64,
    pub collateral_usd_value: u64,
    pub timestamp: i64,
}

#[event]
pub struct ProtocolConfigUpdated {
    pub admin_authority: Pubkey,
    pub paused: bool,
    pub timestamp: i64,
}

#[event]
pub struct ProtocolPaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct ProtocolUnpaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}
// ========== END MEDIUM-SEVERITY FIX (VULN-022) ==========

// ========== SECURITY FIX (VULN-011): USER POSITION COUNTER ==========
#[account]
pub struct UserPositionCounter {
    pub user: Pubkey,
    pub open_positions: u8, // Max 10 positions per user
}

impl UserPositionCounter {
    pub const LEN: usize = 32 + 1; // Pubkey + u8
    pub const MAX_POSITIONS: u8 = 10;
}
// ========== END SECURITY FIX (VULN-011) ==========

#[account]
pub struct ProtocolConfig {
    pub admin_authority: Pubkey,
    pub protocol_paused: bool,
}

impl ProtocolConfig {
    pub const LEN: usize = 32 + 1;
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
    #[msg("Position already matured, use close_at_maturity instead")]
    AlreadyMatured,
    #[msg("Invalid delegate")]
    InvalidDelegate,
    #[msg("Deterministic liquidation threshold breached")]
    DeterministicLiquidationThreshold,
    #[msg("Position is healthy and cannot be liquidated")]
    PositionHealthy,
    #[msg("Invalid admin authority")]
    InvalidAdmin,
    #[msg("Protocol is paused")]
    ProtocolPaused,
    #[msg("Protocol is already paused")]
    AlreadyPaused,  // VULN-020: Circuit breaker
    #[msg("Protocol is not paused")]
    NotPaused,  // VULN-020: Circuit breaker
    #[msg("Invalid LTV parameters")]
    InvalidLtv,
    #[msg("LTV parameters not properly ordered")]
    InvalidLtvOrdering,
    #[msg("LTV too high for safety")]
    LtvTooHigh,
    #[msg("Insufficient liquidation buffer")]
    InsufficientLiquidationBuffer,
    // SECURITY FIX (VULN-004): Oracle price validation errors
    #[msg("Oracle price is stale")]
    OraclePriceStale,
    #[msg("Invalid oracle price")]
    InvalidOraclePrice,
    #[msg("Oracle price out of bounds")]
    OraclePriceOutOfBounds,
    // SECURITY FIX (VULN-007): Minimum position size
    #[msg("Position size too small - minimum $100 collateral and $50 financing required")]
    PositionTooSmall,
    // SECURITY FIX (VULN-010): Oracle source validation
    #[msg("No oracle sources provided")]
    NoOracleSources,
    #[msg("Too many oracle sources (max 3)")]
    TooManyOracleSources,
    #[msg("Invalid oracle source (default/zero address)")]
    InvalidOracleSource,
    // SECURITY FIX (VULN-011): Position limits
    #[msg("User has too many open positions (max 10 per user)")]
    TooManyPositions,
    // SECURITY FIX (VULN-012): Balance validation
    #[msg("Insufficient USDC balance to close position")]
    InsufficientBalanceForClosure,
    // SECURITY FIX (VULN-009): Fee calculation errors
    #[msg("Invalid fee rate - exceeds maximum allowed")]
    InvalidFeeRate,
    #[msg("Fee exceeds collateral amount")]
    FeeExceedsCollateral,
    #[msg("No collateral would be returned to user")]
    NoCollateralReturned,
}

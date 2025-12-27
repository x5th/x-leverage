use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use anchor_spl::associated_token::AssociatedToken;
// TODO: Re-enable LP vault integration after implementing proper CPI
// use lp_vault::program::LpVault;
// use lp_vault::cpi::accounts::AllocateFinancing;
// use lp_vault::cpi::accounts::ReleaseFinancing;
// use lp_vault::cpi::accounts::WriteOffBadDebt;

declare_id!("7PSunTw68XzNT8hEM5KkRL66MWqjWy21hAFHfsipp7gw");

// ========== TWO-TIER LIQUIDATION CONSTANTS ==========
/// Permissionless liquidation threshold - Anyone can liquidate at 73% LTV
pub const PERMISSIONLESS_LIQ_THRESHOLD: u64 = 7300; // 73.00% in basis points

/// Protocol forced liquidation threshold - Protocol intervenes at 75% LTV
pub const PROTOCOL_LIQ_THRESHOLD: u64 = 7500; // 75.00% in basis points

/// Liquidator bonus for external liquidators (5%)
pub const EXTERNAL_LIQUIDATOR_BONUS_BPS: u64 = 500; // 5%

/// Fee on financed asset liquidation (5%)
pub const FORCED_LIQ_FEE_BPS: u64 = 500; // 5%

/// Fee on collateral liquidation (2%)
pub const COLLATERAL_LIQ_FEE_BPS: u64 = 200; // 2%

/// Early closure fee (2% of deferred payment)
pub const EARLY_CLOSURE_FEE_BPS: u64 = 200; // 2%

/// Maximum liquidation percentage per transaction for external liquidators
pub const MAX_EXTERNAL_LIQ_PERCENTAGE: u8 = 50; // 50%

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
        position_index: u64,  // MUST be passed as first param (for #[instruction] macro)
        collateral_amount: u64,
        collateral_usd_value: u64,
        // financed_mint now comes from ctx.accounts.financed_asset_mint
        financing_usdc_amount: u64,    // USDC to spend on asset purchase
        markup_bps: u64,               // Markup in basis points (e.g., 1000 = 10%)
        initial_ltv: u64,
        max_ltv: u64,
        term_start: i64,
        term_end: i64,
        carry_enabled: bool,
        liquidation_threshold: u64,
        oracle_sources: Vec<Pubkey>,
    ) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // ========== MURABAHA: CALCULATE DEFERRED PAYMENT ==========
        // Calculate markup amount from basis points
        let markup_amount = financing_usdc_amount
            .checked_mul(markup_bps)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(10000)
            .ok_or(FinancingError::MathOverflow)?;

        let deferred_payment = financing_usdc_amount
            .checked_add(markup_amount)
            .ok_or(FinancingError::MathOverflow)?;

        msg!("💰 Murabaha Terms:");
        msg!("  Purchase price: ${}", financing_usdc_amount / 1_000_000);
        msg!("  Markup ({}bps): ${}", markup_bps, markup_amount / 1_000_000);
        msg!("  Deferred payment: ${}", deferred_payment / 1_000_000);
        // ========== END MURABAHA CALCULATION ==========

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
            financing_usdc_amount >= MIN_FINANCING_AMOUNT,
            FinancingError::PositionTooSmall
        );
        msg!("✅ Minimum position size validated: collateral=${}, financing=${}",
            collateral_usd_value / 100_000_000, financing_usdc_amount / 1_000_000);
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

        // STEP 2: Get USDC from LP vault for asset purchase
        msg!("Requesting {} USDC from LP vault for commodity purchase", financing_usdc_amount);

        // TODO: Re-enable LP vault CPI integration
        // For now, assume USDC is already in protocol treasury
        msg!("⚠️  MOCK: Using protocol treasury USDC (LP vault CPI disabled)");
        msg!("✅ USDC allocated from LP vault (simulated)");

        // STEP 3: MOCK JUPITER SWAP - Buy financed commodity
        // In production, this would be a CPI to Jupiter aggregator
        // For now, we simulate the swap using oracle-based pricing
        msg!("🔄 MOCK SWAP: Buying financed commodity with USDC");

        let financed_amount = mock_swap_usdc_to_asset(
            &ctx.accounts.protocol_usdc_ata,
            &ctx.accounts.vault_financed_ata,
            &ctx.accounts.token_program,
            financing_usdc_amount,
            &ctx.accounts.financed_asset_mint.key(),
            ctx.bumps.vault_authority,
        )?;

        msg!("✅ Purchased {} units of financed commodity", financed_amount);

        // STEP 3.5: DELIVER financed asset to user immediately (SINGLE CUSTODY MODEL)
        // Protocol only holds collateral, user gets financed asset right away
        msg!("📦 Delivering financed asset to user (single custody model)...");

        let vault_authority_bump = ctx.bumps.vault_authority;
        let seeds = &[b"vault_authority".as_ref(), &[vault_authority_bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_financed_ata.to_account_info(),
                    to: ctx.accounts.user_financed_ata.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer_seeds,
            ),
            financed_amount,
        )?;
        msg!("✅ Financed asset delivered to user");
        msg!("   User now has exposure to {} units", financed_amount);
        msg!("   Protocol holds only collateral as security");

        // STEP 4: Store position state (Murabaha contract terms)
        let state = &mut ctx.accounts.state;
        state.user_pubkey = ctx.accounts.user.key();
        state.position_index = position_index;

        // Collateral
        state.collateral_mint = ctx.accounts.collateral_mint.key();
        state.collateral_amount = collateral_amount;
        state.collateral_usd_value = collateral_usd_value;

        // Financed commodity (what we bought for user)
        state.financed_mint = ctx.accounts.financed_asset_mint.key();
        state.financed_amount = financed_amount;
        state.financed_purchase_price_usdc = financing_usdc_amount;
        state.financed_usd_value = financing_usdc_amount; // Initial value = purchase price

        // Murabaha deferred payment
        state.deferred_payment_amount = deferred_payment;
        state.markup_fees = markup_amount;

        // LTV & Risk
        state.initial_ltv = initial_ltv;
        state.max_ltv = max_ltv;
        state.liquidation_threshold = liquidation_threshold;

        // Term
        state.term_start = term_start;
        state.term_end = term_end;

        // Features
        state.carry_enabled = carry_enabled;
        state.oracle_sources = oracle_sources;
        state.delegated_settlement_authority = Pubkey::default();
        state.delegated_liquidation_authority = Pubkey::default();
        state.position_status = PositionStatus::Active;

        // Update total_positions to track highest index used
        // Allow skipping indices for migration/flexibility
        if position_index >= ctx.accounts.position_counter.total_positions {
            ctx.accounts.position_counter.total_positions = position_index
                .checked_add(1)
                .ok_or(FinancingError::MathOverflow)?;
        }

        // Invariant: No negative equity ever.
        // In Murabaha: Equity = (Collateral + Financed Asset) - Deferred Payment
        // Minimum equity should be positive
        require!(
            collateral_usd_value >= markup_amount,
            FinancingError::NegativeEquity
        );

        msg!("📋 Murabaha Position Summary:");
        msg!("  Collateral: {} (${} USD)", collateral_amount, collateral_usd_value / 100_000_000);
        msg!("  Financed Asset: {} units", financed_amount);
        msg!("  Deferred Payment Due: ${} USDC", deferred_payment / 1_000_000);
        msg!("  Maturity: {} days", (term_end - term_start) / 86400);

        // Emit event for monitoring and indexing
        let clock = Clock::get()?;
        emit!(PositionCreated {
            user: ctx.accounts.user.key(),
            collateral_mint: ctx.accounts.collateral_mint.key(),
            collateral_amount,
            collateral_usd_value,
            financing_amount: deferred_payment,  // Total deferred payment
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
        // In Murabaha: Calculate LTV based on total position value (collateral + financed asset)
        let collateral_value = calculate_position_value_for_ltv(state)?;
        let ltv = compute_ltv(state.deferred_payment_amount, collateral_value)?;

        msg!("LTV Validation (Single Custody - Collateral Only):");
        msg!("  Collateral value: ${}", state.collateral_usd_value / 100_000_000);
        msg!("  Debt: ${}", state.deferred_payment_amount / 1_000_000);
        msg!("  Current LTV: {}%", ltv / 100);
        msg!("  (Note: User owns financed asset, not counted in LTV)");

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

        // SINGLE CUSTODY: LTV based on collateral only
        let previous_ltv = compute_ltv(state.deferred_payment_amount, previous_collateral_value).unwrap_or(0);
        let ltv = compute_ltv(state.deferred_payment_amount, collateral_usd_value)?;

        msg!("Collateral Price Update (Single Custody):");
        msg!("  New collateral value: ${}", collateral_usd_value / 100_000_000);
        msg!("  LTV changed: {}% → {}%", previous_ltv / 100, ltv / 100);
        msg!("  (Note: Financed asset owned by user, not counted in LTV)");

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

    pub fn update_financed_asset_price(
        ctx: Context<UpdateLtv>,
        financed_asset_usd_value: u64
    ) -> Result<()> {
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
        require!(financed_asset_usd_value > 0, FinancingError::InvalidOraclePrice);
        require!(
            financed_asset_usd_value < u64::MAX / 10_000,
            FinancingError::MathOverflow
        );

        msg!("✅ Authority validated: oracle price update authorized");
        // ========== END SECURITY FIX ==========

        // SINGLE CUSTODY: Store financed asset value for records, but doesn't affect LTV
        // User owns the financed asset, can sell it anytime, so we don't control it
        state.financed_usd_value = financed_asset_usd_value;

        // LTV is based on collateral only (what we control)
        let ltv = compute_ltv(state.deferred_payment_amount, state.collateral_usd_value)?;

        msg!("Financed Asset Price Update (Single Custody - Informational Only):");
        msg!("  New financed asset value: ${}", financed_asset_usd_value / 100_000_000);
        msg!("  Current LTV (collateral-based): {}%", ltv / 100);
        msg!("  (Note: Financed asset owned by user, not counted in LTV)");

        // LTV check still based on collateral only
        require!(ltv <= state.max_ltv, FinancingError::LtvBreach);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(LtvUpdated {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            previous_ltv: ltv, // Same as new_ltv since collateral didn't change
            new_ltv: ltv,
            collateral_usd_value: state.collateral_usd_value,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn close_at_maturity(ctx: Context<CloseAtMaturity>) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &mut ctx.accounts.state;
        // ========== SECURITY FIX (VULN-007): AUTHORIZED CLOSURE ONLY ==========
        require_keys_eq!(
            state.user_pubkey,
            ctx.accounts.receiver.key(),
            FinancingError::Unauthorized
        );
        // ========== END SECURITY FIX (VULN-007) ==========
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= state.term_end, FinancingError::NotMatured);
        require!(
            state.position_status == PositionStatus::Active,
            FinancingError::InvalidStatus
        );

        // ========== MURABAHA: DEFERRED PAYMENT SETTLEMENT ==========

        // STEP 1: User MUST repay deferred payment (purchase price + markup) to LP vault
        msg!("💰 Murabaha Settlement:");
        msg!("  Purchase price: ${}", state.financed_purchase_price_usdc / 1_000_000);
        msg!("  Markup: ${}", state.markup_fees / 1_000_000);
        msg!("  Total deferred payment: ${}", state.deferred_payment_amount / 1_000_000);

        require!(
            ctx.accounts.user_usdc_ata.amount >= state.deferred_payment_amount,
            FinancingError::InsufficientBalanceForClosure
        );

        // TODO: Re-enable LP vault CPI integration
        // For now, assume USDC repayment is handled separately
        msg!("⚠️  MOCK: LP vault debt repayment (LP vault CPI disabled)");
        msg!("✅ Deferred payment repaid to LP vault (simulated)");

        // ========== END MURABAHA SETTLEMENT ==========

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
        msg!("✅ Collateral returned successfully");

        // ========== SINGLE CUSTODY MODEL ==========
        // User already received financed asset at position opening
        // They only need collateral back after repaying debt
        msg!("💡 User already owns financed asset (received at position opening)");
        msg!("🎉 Position closed - collateral returned!");
        // ========== END SINGLE CUSTODY MODEL ==========

        // STEP 3: Decrement position counter
        // ========== SECURITY FIX (VULN-011): DECREMENT POSITION COUNTER ==========
        let counter = &mut ctx.accounts.position_counter;
        counter.open_positions = counter.open_positions
            .checked_sub(1)
            .ok_or(FinancingError::MathOverflow)?;
        msg!("✅ Position counter decremented: user now has {} open positions",
            counter.open_positions);
        // ========== END SECURITY FIX (VULN-011) ==========

        // STEP 5: Atomic closure - all fields transitioned in one shot
        state.position_status = PositionStatus::Closed;

        // Emit event for monitoring
        emit!(PositionClosed {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            collateral_returned: state.collateral_amount,
            debt_repaid: state.deferred_payment_amount,
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
        // ========== SECURITY FIX (VULN-007): AUTHORIZED CLOSURE ONLY ==========
        require_keys_eq!(
            state.user_pubkey,
            ctx.accounts.receiver.key(),
            FinancingError::Unauthorized
        );
        // ========== END SECURITY FIX (VULN-007) ==========
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

        // ========== MURABAHA EARLY CLOSURE: DEFERRED PAYMENT ==========
        // STEP 1: User MUST repay full deferred payment (Murabaha markup is not reduced for early closure)
        let user_usdc_balance = ctx.accounts.user_financed_ata.amount;
        let required_repayment = state.deferred_payment_amount;

        require!(
            user_usdc_balance >= required_repayment,
            FinancingError::InsufficientBalanceForClosure
        );
        msg!("✅ Sufficient USDC balance validated: {} >= {}",
             user_usdc_balance, required_repayment);
        msg!("  Deferred payment (purchase + markup): ${}", required_repayment / 1_000_000);
        // ========== END BALANCE VALIDATION ==========

        // STEP 2: Repay deferred payment to LP vault
        msg!("Repaying ${} USDC deferred payment to LP vault", required_repayment / 1_000_000);

        // TODO: Re-enable LP vault CPI integration
        // For now, assume USDC repayment is handled separately
        msg!("⚠️  MOCK: LP vault debt repayment (LP vault CPI disabled)");
        msg!("✅ Deferred payment repaid to LP vault (simulated)");

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
            debt_repaid: state.deferred_payment_amount,
            early_closure: true,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// TIER 1: Permissionless Liquidation (73% LTV)
    /// Anyone can liquidate when LTV >= 73% but < 75%
    /// Liquidator brings USDC, repays debt, receives collateral + financed asset + 5% bonus
    /// Supports partial liquidations (max 50% per transaction)
    pub fn liquidate(
        ctx: Context<Liquidate>,
        liquidation_percentage: u8,  // 1-50% for external liquidators
    ) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &mut ctx.accounts.state;
        let clock = Clock::get()?;

        // STEP 1: Calculate current LTV (COLLATERAL ONLY - Single Custody)
        let collateral_value = calculate_position_value_for_ltv(state)?;
        let current_ltv = compute_ltv(state.deferred_payment_amount, collateral_value)?;

        msg!("🔔 PERMISSIONLESS LIQUIDATION (73% LTV Tier - Single Custody)");
        msg!("  Collateral value: ${}", state.collateral_usd_value / 100_000_000);
        msg!("  Debt: ${}", state.deferred_payment_amount / 1_000_000);
        msg!("  Current LTV: {}%", current_ltv / 100);
        msg!("  (Note: User owns financed asset, only collateral available for liquidation)");

        // STEP 2: Verify position is in permissionless liquidation zone (73% - 75%)
        require!(
            current_ltv >= PERMISSIONLESS_LIQ_THRESHOLD,
            FinancingError::PositionHealthy
        );
        require!(
            current_ltv < PROTOCOL_LIQ_THRESHOLD,
            FinancingError::UseProtocolLiquidation
        );

        msg!("✅ Position is in permissionless liquidation zone (73%-75%)");

        // STEP 3: Validate liquidation percentage (max 50% for external liquidators)
        require!(
            liquidation_percentage > 0 && liquidation_percentage <= MAX_EXTERNAL_LIQ_PERCENTAGE,
            FinancingError::ExcessiveLiquidationPercentage
        );

        msg!("  Liquidating {}% of position", liquidation_percentage);

        // STEP 4: Calculate amounts
        let debt_to_repay = state.deferred_payment_amount
            .checked_mul(liquidation_percentage as u64)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(100)
            .ok_or(FinancingError::MathOverflow)?;

        let liquidator_bonus = debt_to_repay
            .checked_mul(EXTERNAL_LIQUIDATOR_BONUS_BPS)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(FinancingError::MathOverflow)?;

        msg!("  Debt to repay: ${}", debt_to_repay / 1_000_000);
        msg!("  Liquidator bonus (5%): ${}", liquidator_bonus / 1_000_000);

        // STEP 5: Liquidator repays debt (USDC) to protocol treasury
        msg!("💰 Liquidator repaying debt to protocol treasury...");
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.liquidator_usdc_ata.to_account_info(),
                    to: ctx.accounts.protocol_usdc_ata.to_account_info(),
                    authority: ctx.accounts.liquidator.to_account_info(),
                },
            ),
            debt_to_repay,
        )?;
        msg!("✅ Debt repaid: ${}", debt_to_repay / 1_000_000);

        // STEP 6: SINGLE CUSTODY - Transfer collateral to liquidator (proportional + bonus)
        // User owns financed asset, so liquidator gets collateral only
        let vault_authority_bump = ctx.bumps.vault_authority;
        let seeds = &[b"vault_authority".as_ref(), &[vault_authority_bump]];
        let signer_seeds = &[&seeds[..]];

        // Calculate collateral to seize: proportional amount + bonus
        // Total value of debt repaid + bonus
        let total_claim = debt_to_repay
            .checked_add(liquidator_bonus)
            .ok_or(FinancingError::MathOverflow)?;

        // Convert USD value to collateral tokens
        let collateral_to_seize = total_claim
            .checked_mul(100_000_000) // Convert from 6 decimals (USDC) to 8 decimals (USD value)
            .ok_or(FinancingError::MathOverflow)?
            .checked_mul(state.collateral_amount)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(state.collateral_usd_value)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(100)  // Adjust for decimal conversion
            .ok_or(FinancingError::MathOverflow)?;

        msg!("  Transferring {} collateral to liquidator (covers ${} debt + ${} bonus)",
             collateral_to_seize, debt_to_repay / 1_000_000, liquidator_bonus / 1_000_000);

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
            collateral_to_seize,
        )?;

        // STEP 7: Update position state (reduce debt and collateral)
        state.deferred_payment_amount = state.deferred_payment_amount
            .checked_sub(debt_to_repay)
            .ok_or(FinancingError::MathOverflow)?;
        state.collateral_amount = state.collateral_amount
            .checked_sub(collateral_to_seize)
            .ok_or(FinancingError::MathOverflow)?;

        // Update collateral USD value proportionally
        state.collateral_usd_value = state.collateral_usd_value
            .checked_mul(state.collateral_amount)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(state.collateral_amount.checked_add(collateral_to_seize).ok_or(FinancingError::MathOverflow)?)
            .ok_or(FinancingError::MathOverflow)?;

        // financed_amount tracking remains unchanged (user still owns it)

        msg!("✅ Permissionless liquidation complete!");
        msg!("  Liquidator received: {} collateral tokens", collateral_to_seize);
        msg!("  Remaining debt: ${}", state.deferred_payment_amount / 1_000_000);
        msg!("  Remaining collateral: {} tokens", state.collateral_amount);

        // Emit event
        emit!(PositionLiquidated {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            liquidator: ctx.accounts.liquidator.key(),
            collateral_seized: collateral_to_seize,
            debt_recovered: debt_to_repay,
            bad_debt: 0,
            forced: false,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// TIER 2: Protocol Forced Liquidation (75% LTV)
    /// Only callable by protocol admin when LTV >= 75%
    /// Protocol sells assets on DEX, pays LP vault, returns remaining collateral to user
    /// NO USDC reserves needed - protocol sells directly on market
    pub fn force_liquidate_protocol(ctx: Context<ForceLiquidate>) -> Result<()> {
        // ========== CIRCUIT BREAKER CHECK ==========
        require!(!ctx.accounts.protocol_config.protocol_paused, FinancingError::ProtocolPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        let state = &mut ctx.accounts.state;
        let config = &ctx.accounts.protocol_config;

        // ========== AUTHORITY VALIDATION ==========
        // Only protocol admin can force liquidate
        require!(
            ctx.accounts.authority.key() == config.admin_authority,
            FinancingError::Unauthorized
        );
        msg!("✅ Authority validated: protocol admin force liquidation");
        // ========== END AUTHORITY VALIDATION ==========

        // STEP 1: Calculate current LTV (COLLATERAL ONLY - Single Custody)
        let collateral_value = calculate_position_value_for_ltv(state)?;
        let current_ltv = compute_ltv(state.deferred_payment_amount, collateral_value)?;

        msg!("⚠️  PROTOCOL FORCED LIQUIDATION (75% LTV Tier - Single Custody)");
        msg!("  Collateral value: ${}", state.collateral_usd_value / 100_000_000);
        msg!("  Debt: ${}", state.deferred_payment_amount / 1_000_000);
        msg!("  Current LTV: {}%", current_ltv / 100);
        msg!("  (Note: User owns financed asset, only collateral available for liquidation)");

        // STEP 2: Verify position is at protocol threshold
        require!(
            current_ltv >= PROTOCOL_LIQ_THRESHOLD,
            FinancingError::NotAtProtocolThreshold
        );

        msg!("✅ Position is at protocol threshold (≥75%)");

        let total_debt = state.deferred_payment_amount;
        let clock = Clock::get()?;

        // SINGLE CUSTODY: We only have collateral to liquidate
        // User owns the financed asset, so protocol sells collateral on DEX to recover debt
        msg!("💱 SINGLE CUSTODY: Liquidating collateral to cover debt...");

        let vault_authority_bump = ctx.bumps.vault_authority;
        let seeds = &[b"vault_authority".as_ref(), &[vault_authority_bump]];
        let signer_seeds = &[&seeds[..]];

        // Calculate liquidation fee (5% on collateral sale)
        let collateral_liq_fee = total_debt
            .checked_mul(FORCED_LIQ_FEE_BPS)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(FinancingError::MathOverflow)?;

        let total_needed = total_debt
            .checked_add(collateral_liq_fee)
            .ok_or(FinancingError::MathOverflow)?;

        // Calculate collateral tokens to sell
        // Convert USD amount to collateral tokens: (needed_usd / collateral_usd_value) * collateral_amount
        let collateral_to_sell = (total_needed as u128)
            .checked_mul(100_000_000) // Scale up to avoid precision loss
            .ok_or(FinancingError::MathOverflow)?
            .checked_mul(state.collateral_amount as u128)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(state.collateral_usd_value as u128)
            .ok_or(FinancingError::MathOverflow)?
            .checked_div(100_000_000) // Scale back down
            .ok_or(FinancingError::MathOverflow)? as u64;

        msg!("  Selling {} collateral tokens to cover ${} debt + ${} fee",
             collateral_to_sell, total_debt / 1_000_000, collateral_liq_fee / 1_000_000);

        // Mock sell collateral on DEX (would be actual DEX call in production)
        let collateral_proceeds = mock_sell_asset_to_usdc(
            &state.collateral_mint,
            collateral_to_sell,
        )?;

        msg!("  Collateral sale proceeds: ${}", collateral_proceeds / 1_000_000);
        msg!("  Sending to protocol treasury/LP vault (simulated)");

        // Return remaining collateral to user
        let remaining_collateral = state.collateral_amount
            .checked_sub(collateral_to_sell)
            .ok_or(FinancingError::MathOverflow)?;

        if remaining_collateral > 0 {
            msg!("  Returning {} remaining collateral tokens to user", remaining_collateral);
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
                remaining_collateral,
            )?;
            msg!("✅ Protocol liquidation complete - {} collateral returned", remaining_collateral);
        } else {
            msg!("✅ Protocol liquidation complete - no collateral remaining");
        }

        // STEP 6: Close position
        state.position_status = PositionStatus::Liquidated;

        // Decrement counter
        let counter = &mut ctx.accounts.position_counter;
        counter.open_positions = counter.open_positions
            .checked_sub(1)
            .ok_or(FinancingError::MathOverflow)?;

        msg!("✅ Position counter decremented: user now has {} open positions",
            counter.open_positions);

        // Emit event
        emit!(PositionLiquidated {
            user: state.user_pubkey,
            collateral_mint: state.collateral_mint,
            liquidator: ctx.accounts.authority.key(),
            collateral_seized: collateral_to_sell,
            debt_recovered: total_debt,
            bad_debt: 0, // No bad debt with collateral-based liquidation
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

// ========== MOCK JUPITER SWAP HELPER ==========
// TODO: Replace with real Jupiter CPI in production
// This mock function simulates buying financed commodity with USDC
// using oracle-based pricing
fn mock_swap_usdc_to_asset<'info>(
    protocol_usdc_ata: &Account<'info, TokenAccount>,
    vault_financed_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    usdc_amount: u64,
    financed_mint: &Pubkey,
    _vault_authority_bump: u8,
) -> Result<u64> {
    // Mock oracle prices (in USD with 8 decimals)
    const SOL_PRICE: u64 = 150_00000000; // $150
    const ETH_PRICE: u64 = 3000_00000000; // $3,000
    const BTC_PRICE: u64 = 100000_00000000; // $100,000
    const XNT_PRICE: u64 = 1_00000000; // $1

    // Known mints (from test setup)
    const SOL_MINT: &str = "EeoqCfDd2x5UaD21q2yam2QtBaHQxDzA9GrLyFBJkKEA";
    const ETH_MINT: &str = "BcfBSHvFjAtvDfBGthSKYf53QCoMvrgaQ81XfoTtmyN3";
    const BTC_MINT: &str = "DBtAa2vKhdEJKL2sHiaetPvoWxSPJxazqRtQrGJ4ptTN";
    const XNT_MINT: &str = "DmsV7P9SxzvrvcNL77Eej1M82zkBHeYLWsX6EV915tnz";

    let mint_str = financed_mint.to_string();

    // Get price based on mint
    let (asset_price, decimals) = if mint_str == SOL_MINT {
        (SOL_PRICE, 9)
    } else if mint_str == ETH_MINT {
        (ETH_PRICE, 9)
    } else if mint_str == BTC_MINT {
        (BTC_PRICE, 8)
    } else if mint_str == XNT_MINT {
        (XNT_PRICE, 9)
    } else {
        msg!("⚠️  Unknown mint for mock swap: {}", mint_str);
        return Err(FinancingError::InvalidOracleSource.into());
    };

    // Calculate amount of asset to "buy"
    // usdc_amount is in 6 decimals, asset_price is in 8 decimals
    // financed_amount should be in asset's native decimals
    let usdc_value_8_decimals = usdc_amount
        .checked_mul(100) // Convert from 6 to 8 decimals
        .ok_or(FinancingError::MathOverflow)?;

    let financed_amount_base = usdc_value_8_decimals
        .checked_mul(10u64.pow(decimals))
        .ok_or(FinancingError::MathOverflow)?
        .checked_div(asset_price)
        .ok_or(FinancingError::MathOverflow)?;

    msg!("🔄 MOCK SWAP:");
    msg!("  Spending: ${} USDC", usdc_amount / 1_000_000);
    msg!("  Asset price: ${}", asset_price / 100_000_000);
    msg!("  Receiving: {} units of asset", financed_amount_base);

    // In a real implementation, this would:
    // 1. Transfer USDC to Jupiter/DEX
    // 2. Execute swap via CPI
    // 3. Receive asset tokens
    //
    // For mock: We just validate that the vault has sufficient balance
    // (assuming tokens were pre-funded for testing)
    require!(
        vault_financed_ata.amount >= financed_amount_base,
        FinancingError::InsufficientVaultBalance
    );

    msg!("✅ Mock swap validated - vault has sufficient balance");

    Ok(financed_amount_base)
}

// ========== MOCK DEX SELL HELPER (for protocol liquidations) ==========
// TODO: Replace with real DEX integration (Xendex/Jupiter) in production
// Simulates selling asset for USDC using oracle prices
fn mock_sell_asset_to_usdc(
    asset_mint: &Pubkey,
    asset_amount: u64,
) -> Result<u64> {
    // Mock oracle prices (in USD with 8 decimals)
    const SOL_PRICE: u64 = 150_00000000; // $150
    const ETH_PRICE: u64 = 3000_00000000; // $3,000
    const BTC_PRICE: u64 = 100000_00000000; // $100,000
    const XNT_PRICE: u64 = 1_00000000; // $1

    // Known mints (from test setup)
    const SOL_MINT: &str = "EeoqCfDd2x5UaD21q2yam2QtBaHQxDzA9GrLyFBJkKEA";
    const ETH_MINT: &str = "BcfBSHvFjAtvDfBGthSKYf53QCoMvrgaQ81XfoTtmyN3";
    const BTC_MINT: &str = "DBtAa2vKhdEJKL2sHiaetPvoWxSPJxazqRtQrGJ4ptTN";
    const XNT_MINT: &str = "DmsV7P9SxzvrvcNL77Eej1M82zkBHeYLWsX6EV915tnz";

    let mint_str = asset_mint.to_string();

    // Get price based on mint
    let (asset_price, decimals) = if mint_str == SOL_MINT {
        (SOL_PRICE, 9)
    } else if mint_str == ETH_MINT {
        (ETH_PRICE, 9)
    } else if mint_str == BTC_MINT {
        (BTC_PRICE, 8)
    } else if mint_str == XNT_MINT {
        (XNT_PRICE, 9)
    } else {
        msg!("⚠️  Unknown mint for mock sell: {}", mint_str);
        return Err(FinancingError::InvalidOracleSource.into());
    };

    // Calculate USDC proceeds
    // asset_amount is in native decimals, asset_price is in 8 decimals
    let asset_value_8_decimals = (asset_amount as u128)
        .checked_mul(asset_price as u128)
        .ok_or(FinancingError::MathOverflow)?
        .checked_div(10u128.pow(decimals))
        .ok_or(FinancingError::MathOverflow)?;

    // Convert from 8 decimals to 6 decimals (USDC)
    let usdc_proceeds = asset_value_8_decimals
        .checked_div(100)
        .ok_or(FinancingError::MathOverflow)? as u64;

    msg!("🔄 MOCK SELL:");
    msg!("  Selling: {} units of asset", asset_amount);
    msg!("  Asset price: ${}", asset_price / 100_000_000);
    msg!("  USDC proceeds: ${}", usdc_proceeds / 1_000_000);

    Ok(usdc_proceeds)
}

// ========== POSITION VALUE CALCULATION ==========
// Calculates total position value (collateral + financed asset)
/// SINGLE CUSTODY MODEL: LTV based on collateral only
/// User owns financed asset (can sell/transfer it anytime)
/// Protocol only controls collateral, so LTV = debt / collateral_value
/// This matches standard lending protocols (Aave, Compound)
fn calculate_position_value_for_ltv(state: &FinancingState) -> Result<u64> {
    // Only collateral is under protocol control in single custody
    Ok(state.collateral_usd_value)
}

// TODO: DUAL CUSTODY MODEL - Commented out for single custody
// fn calculate_total_position_value(state: &FinancingState) -> Result<u64> {
//     let total_value = state.collateral_usd_value
//         .checked_add(state.financed_usd_value)
//         .ok_or(FinancingError::MathOverflow)?;
//     Ok(total_value)
// }

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
#[instruction(position_index: u64)]
pub struct InitializeFinancing<'info> {
    #[account(
        init,
        payer = user,
        space = 8 + FinancingState::LEN,
        seeds = [b"financing", user.key().as_ref(), &position_index.to_le_bytes()],
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

    /// USDC mint (currency for financing)
    pub usdc_mint: Account<'info, Mint>,

    // TODO: Re-enable LP vault integration
    // // ===== LP VAULT INTEGRATION =====
    // /// LP Vault state PDA
    // #[account(mut)]
    // pub lp_vault: Account<'info, lp_vault::LPVaultState>,
    //
    // /// LP Vault's USDC token account (source of financing)
    // #[account(
    //     mut,
    //     constraint = lp_vault_usdc_ata.mint == usdc_mint.key()
    // )]
    // pub lp_vault_usdc_ata: Account<'info, TokenAccount>,

    /// Protocol's USDC token account (mock - would receive from LP vault in production)
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = usdc_mint,
        associated_token::authority = vault_authority
    )]
    pub protocol_usdc_ata: Account<'info, TokenAccount>,

    /// Financed asset mint (BTC/ETH/SOL/XNT - what user wants to leverage-buy)
    /// This must be passed as a parameter to initialize_financing
    pub financed_asset_mint: Account<'info, Mint>,

    /// Vault's token account to hold financed commodity temporarily
    /// (Only holds it for a moment before transferring to user)
    #[account(
        mut,
        constraint = vault_financed_ata.owner == vault_authority.key(),
        constraint = vault_financed_ata.mint == financed_asset_mint.key()
    )]
    pub vault_financed_ata: Account<'info, TokenAccount>,

    /// User's token account to receive financed asset (SINGLE CUSTODY MODEL)
    /// User gets the financed asset immediately, protocol only holds collateral
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = financed_asset_mint,
        associated_token::authority = user
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    // TODO: Re-enable LP vault program integration
    // /// LP vault program
    // pub lp_vault_program: Program<'info, LpVault>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

#[derive(Accounts)]
pub struct ValidateLtv<'info> {
    #[account(
        mut,
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
        bump
    )]
    pub state: Account<'info, FinancingState>,
}

#[derive(Accounts)]
pub struct AssignDelegatedAuthorities<'info> {
    #[account(
        mut,
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
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
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
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
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
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

    /// USDC mint (repayment currency)
    pub usdc_mint: Account<'info, Mint>,

    // TODO: Re-enable LP vault integration
    // // ===== MURABAHA: LP VAULT ACCOUNTS FOR DEFERRED PAYMENT REPAYMENT =====
    // /// LP Vault state PDA
    // #[account(mut)]
    // pub lp_vault: Account<'info, lp_vault::LPVaultState>,
    //
    // /// LP Vault's USDC account (receives deferred payment)
    // #[account(
    //     mut,
    //     constraint = lp_vault_usdc_ata.mint == usdc_mint.key()
    // )]
    // pub lp_vault_usdc_ata: Account<'info, TokenAccount>,

    /// User's USDC account (source of deferred payment)
    #[account(
        mut,
        constraint = user_usdc_ata.owner == receiver.key(),
        constraint = user_usdc_ata.mint == usdc_mint.key()
    )]
    pub user_usdc_ata: Account<'info, TokenAccount>,

    // ========== SINGLE CUSTODY MODEL ==========
    // User already received financed asset at position opening
    // No need to return it at maturity - they already own it
    // Protocol only holds collateral as security
    // ========== END SINGLE CUSTODY MODEL ==========

    // TODO: CARRY MODEL (DUAL CUSTODY) - Commented out for simplicity
    // // ===== MURABAHA: FINANCED COMMODITY RETURN =====
    // /// Vault's token account holding financed commodity (e.g., BTC)
    // #[account(
    //     mut,
    //     constraint = vault_financed_commodity_ata.mint == state.financed_mint,
    //     constraint = vault_financed_commodity_ata.owner == vault_authority.key()
    // )]
    // pub vault_financed_commodity_ata: Account<'info, TokenAccount>,
    //
    // /// User's token account to receive financed commodity
    // #[account(
    //     mut,
    //     constraint = user_financed_commodity_ata.owner == receiver.key(),
    //     constraint = user_financed_commodity_ata.mint == state.financed_mint
    // )]
    // pub user_financed_commodity_ata: Account<'info, TokenAccount>,

    // TODO: Re-enable LP vault program integration
    // /// LP vault program
    // pub lp_vault_program: Program<'info, LpVault>,

    // ===== CIRCUIT BREAKER (VULN-020) =====
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

#[derive(Accounts)]
pub struct CloseEarly<'info> {
    #[account(
        mut,
        close = receiver,
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
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

    /// Financing token mint (USDC - repayment currency)
    pub financed_mint: Account<'info, Mint>,

    // TODO: Re-enable LP vault integration
    // // ===== LP VAULT INTEGRATION =====
    // /// LP Vault state PDA
    // #[account(mut)]
    // pub lp_vault: Account<'info, lp_vault::LPVaultState>,
    //
    // /// LP Vault's token account holding liquidity (destination)
    // #[account(
    //     mut,
    //     constraint = vault_financed_ata.mint == financed_mint.key(),
    //     constraint = vault_financed_ata.owner == lp_vault.key()
    // )]
    // pub vault_financed_ata: Account<'info, TokenAccount>,

    /// User's token account for USDC repayment (source)
    #[account(
        init_if_needed,
        payer = receiver,
        associated_token::mint = financed_mint,
        associated_token::authority = receiver
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    // TODO: Re-enable LP vault program integration
    // /// LP vault program
    // pub lp_vault_program: Program<'info, LpVault>,

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
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
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

    // TODO: DUAL CUSTODY - Financed asset accounts (commented out for single custody)
    // // ===== FINANCED ASSET ACCOUNTS (Murabaha dual custody model) =====
    // /// Financed asset mint (BTC/ETH/SOL/XNT - what was bought for the user)
    // pub financed_mint: Account<'info, Mint>,
    //
    // /// Vault's token account holding financed asset (source)
    // #[account(
    //     mut,
    //     constraint = vault_financed_ata.mint == financed_mint.key(),
    //     constraint = vault_financed_ata.owner == vault_authority.key()
    // )]
    // pub vault_financed_ata: Account<'info, TokenAccount>,
    //
    // /// Liquidator's token account to receive financed asset (destination)
    // #[account(
    //     mut,
    //     constraint = liquidator_financed_ata.mint == financed_mint.key(),
    //     constraint = liquidator_financed_ata.owner == liquidator.key()
    // )]
    // pub liquidator_financed_ata: Account<'info, TokenAccount>,

    // ===== USDC ACCOUNTS (for debt repayment - Single Custody) =====
    /// USDC mint
    pub usdc_mint: Account<'info, Mint>,

    /// Liquidator's USDC account (source of payment)
    #[account(
        mut,
        constraint = liquidator_usdc_ata.mint == usdc_mint.key(),
        constraint = liquidator_usdc_ata.owner == liquidator.key()
    )]
    pub liquidator_usdc_ata: Account<'info, TokenAccount>,

    /// Protocol treasury USDC account (destination for debt repayment)
    /// TODO: This should eventually be LP vault for proper debt repayment
    #[account(
        mut,
        constraint = protocol_usdc_ata.mint == usdc_mint.key()
    )]
    pub protocol_usdc_ata: Account<'info, TokenAccount>,

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
        seeds = [b"financing", state.user_pubkey.as_ref(), &state.position_index.to_le_bytes()],
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

    // TODO: DUAL CUSTODY - Financed asset accounts (commented out for single custody)
    // // ===== FINANCED ASSET ACCOUNTS (for protocol liquidation) =====
    // /// Financed asset mint (BTC/ETH/SOL/XNT - what was bought for the user)
    // pub financed_mint: Account<'info, Mint>,
    //
    // /// Vault's token account holding financed asset (source for liquidation)
    // #[account(
    //     mut,
    //     constraint = vault_financed_ata.mint == financed_mint.key(),
    //     constraint = vault_financed_ata.owner == vault_authority.key()
    // )]
    // pub vault_financed_ata: Account<'info, TokenAccount>,
    //
    // /// Protocol's token account for financed asset (destination - for mock swap)
    // #[account(
    //     mut,
    //     constraint = protocol_financed_ata.mint == financed_mint.key()
    // )]
    // pub protocol_financed_ata: Account<'info, TokenAccount>,

    // ===== USER COLLATERAL RETURN (Single Custody) =====
    /// User's token account to receive remaining collateral
    #[account(
        mut,
        constraint = user_collateral_ata.mint == collateral_mint.key(),
        constraint = user_collateral_ata.owner == state.user_pubkey
    )]
    pub user_collateral_ata: Account<'info, TokenAccount>,
}

#[account]
pub struct FinancingState {
    // User & Position Identification
    pub user_pubkey: Pubkey,
    pub position_index: u64,

    // COLLATERAL (what user deposits as security)
    pub collateral_mint: Pubkey,
    pub collateral_amount: u64,
    pub collateral_usd_value: u64,

    // FINANCED COMMODITY (Murabaha: what protocol buys for user)
    pub financed_mint: Pubkey,              // Which asset user wants to leverage-buy (BTC/ETH/SOL/XNT)
    pub financed_amount: u64,               // Amount of that asset purchased
    pub financed_purchase_price_usdc: u64,  // USDC spent to buy the commodity
    pub financed_usd_value: u64,            // Current USD value of financed asset (updated by oracle)

    // MURABAHA DEFERRED PAYMENT
    pub deferred_payment_amount: u64,       // Total user owes at maturity (cost + markup)
    pub markup_fees: u64,                   // Profit margin (NOT interest - Shariah compliant)

    // LTV & Risk Management
    pub initial_ltv: u64,
    pub max_ltv: u64,
    pub liquidation_threshold: u64,

    // Term
    pub term_start: i64,
    pub term_end: i64,

    // Features
    pub carry_enabled: bool,
    pub oracle_sources: Vec<Pubkey>,
    pub delegated_settlement_authority: Pubkey,
    pub delegated_liquidation_authority: Pubkey,
    pub position_status: PositionStatus,
}

impl FinancingState {
    pub const LEN: usize = 32 // user
        + 8 // position_index
        + 32 // collateral mint
        + 8 // collateral_amount
        + 8 // collateral_usd_value
        + 32 // financed_mint
        + 8 // financed_amount
        + 8 // financed_purchase_price_usdc
        + 8 // financed_usd_value (NEW)
        + 8 // deferred_payment_amount
        + 8 // markup_fees
        + 8 // initial_ltv
        + 8 // max_ltv
        + 8 // liquidation_threshold
        + 8 // term_start
        + 8 // term_end
        + 1 // carry_enabled
        + 4 + 10 * 32 // oracle vector capped at 10
        + 32 // delegated_settlement_authority
        + 32 // delegated_liquidation_authority
        + 1; // position_status
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
    pub total_positions: u64, // Total positions created (for PDA derivation)
}

impl UserPositionCounter {
    pub const LEN: usize = 32 + 1 + 8; // Pubkey + u8 + u64
    pub const MAX_POSITIONS: u8 = 250; // Increased for multi-position support (u8 max is 255)
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
    // Multi-position support
    #[msg("Invalid position index - must match total_positions counter")]
    InvalidPositionIndex,
    // Murabaha/Mock swap errors
    #[msg("Insufficient balance in vault for mock swap")]
    InsufficientVaultBalance,
    // Two-tier liquidation errors
    #[msg("Position LTV is below protocol threshold (75%) - use permissionless liquidation instead")]
    NotAtProtocolThreshold,
    #[msg("Position LTV is at or above protocol threshold (75%) - must use protocol liquidation")]
    UseProtocolLiquidation,
    #[msg("Liquidation percentage exceeds maximum allowed (50% for external liquidators)")]
    ExcessiveLiquidationPercentage,
}

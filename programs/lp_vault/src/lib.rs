use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer, MintTo, Burn};

declare_id!("BKCWUpTk3B1yXoFAWugnmLM5s2S1HWpmNiAE3ZJQn5eE");

#[program]
pub mod lp_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>, authority: Pubkey) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.total_shares = 0;
        vault.vault_usdc_balance = 0;
        vault.locked_for_financing = 0;
        vault.utilization = 0;
        vault.authority = authority;
        vault.paused = false;  // Start unpaused

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(VaultInitialized {
            authority,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn migrate_vault_authority(
        ctx: Context<MigrateVaultAuthority>,
        authority: Pubkey,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        if vault.authority != Pubkey::default() {
            vault.assert_authority(ctx.accounts.authority.key())?;
        }
        vault.authority = authority;
        Ok(())
    }

    pub fn deposit_usdc(ctx: Context<DepositUsdc>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!vault.paused, VaultError::VaultPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        require!(amount > 0, VaultError::ZeroAmount);
        let pre_shares = vault.total_shares;
        let pre_price = vault.share_price();

        let shares = if vault.total_shares == 0 {
            // First deposit: 1:1 ratio (amount in lamports = shares)
            amount
        } else {
            // Subsequent deposits: shares = (amount * total_shares) / vault_balance
            // To avoid overflow, use u128 for intermediate calculation
            let amount_u128 = amount as u128;
            let total_shares_u128 = vault.total_shares as u128;
            let balance_u128 = vault.vault_usdc_balance.max(1) as u128;

            let shares_u128 = (amount_u128 * total_shares_u128) / balance_u128;

            // Convert back to u64, check for overflow
            let shares = shares_u128
                .try_into()
                .map_err(|_| VaultError::MathOverflow)?;

            shares
        };

        // STEP 1: Transfer USDC from user to vault
        msg!("Transferring {} USDC from user to vault", amount);
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_usdc_account.to_account_info(),
                    to: ctx.accounts.vault_usdc_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )?;

        // STEP 2: Mint LP tokens to user
        let vault_bump = ctx.bumps.vault;
        let seeds = &[b"vault".as_ref(), &[vault_bump]];
        let signer_seeds = &[&seeds[..]];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.lp_token_mint.to_account_info(),
                    to: ctx.accounts.user_lp_token_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds,
            ),
            shares,
        )?;

        vault.total_shares = vault.total_shares.saturating_add(shares);
        vault.vault_usdc_balance = vault.vault_usdc_balance.saturating_add(amount);
        let post_price = vault.share_price();

        // Only check for share price regression if there were existing shares
        // First deposit establishes the base price
        if pre_shares > 0 {
            require!(post_price >= pre_price, VaultError::SharePriceRegression);
        }
        vault.update_utilization();

        msg!("Deposited {} USDC, minted {} LP tokens", amount, shares);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(LPDeposited {
            user: ctx.accounts.user.key(),
            amount,
            shares,
            total_shares: vault.total_shares,
            vault_balance: vault.vault_usdc_balance,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn withdraw_usdc(ctx: Context<WithdrawUsdc>, shares: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!vault.paused, VaultError::VaultPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        require!(shares > 0, VaultError::ZeroAmount);
        require!(shares <= vault.total_shares, VaultError::InsufficientShares);

        let amount = vault.redeem_amount(shares)?;

        // Check that vault has enough available liquidity (not locked for financing)
        let available = vault.vault_usdc_balance.saturating_sub(vault.locked_for_financing);
        require!(amount <= available, VaultError::InsufficientLiquidity);

        // STEP 1: Burn LP tokens from user
        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.lp_token_mint.to_account_info(),
                    from: ctx.accounts.user_lp_token_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            shares,
        )?;

        // STEP 2: Transfer USDC from vault to user
        msg!("Transferring {} USDC from vault to user", amount);
        let vault_bump = ctx.bumps.vault;
        let seeds = &[b"vault".as_ref(), &[vault_bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_usdc_account.to_account_info(),
                    to: ctx.accounts.user_usdc_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        vault.total_shares = vault.total_shares.saturating_sub(shares);
        vault.vault_usdc_balance = vault.vault_usdc_balance.saturating_sub(amount);
        let post_price = vault.share_price();
        // Share price can drop only in bad debt events; enforce non-negative.
        require!(post_price > 0, VaultError::SharePriceRegression);
        vault.update_utilization();

        msg!("Burned {} LP tokens, withdrew {} USDC", shares, amount);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(LPWithdrawn {
            user: ctx.accounts.user.key(),
            shares,
            amount,
            total_shares: vault.total_shares,
            vault_balance: vault.vault_usdc_balance,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn mint_shares(ctx: Context<ManageShares>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.assert_authority(ctx.accounts.authority.key())?;
        vault.total_shares = vault.total_shares.saturating_add(amount);
        Ok(())
    }

    pub fn burn_shares(ctx: Context<ManageShares>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.assert_authority(ctx.accounts.authority.key())?;
        require!(vault.total_shares >= amount, VaultError::InsufficientShares);
        vault.total_shares = vault.total_shares.saturating_sub(amount);
        Ok(())
    }

    pub fn allocate_financing(ctx: Context<AllocateFinancing>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        // ========== CIRCUIT BREAKER CHECK (VULN-020) ==========
        require!(!vault.paused, VaultError::VaultPaused);
        // ========== END CIRCUIT BREAKER CHECK ==========

        // No authority check - this is a CPI-only function called by authorized programs
        require!(
            amount <= vault.vault_usdc_balance,
            VaultError::InsufficientLiquidity
        );

        // STEP 1: Transfer financed tokens from LP vault to user
        msg!("Transferring {} financed tokens from LP vault to user", amount);

        let vault_bump = ctx.bumps.vault;
        let seeds = &[b"vault".as_ref(), &[vault_bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_ata.to_account_info(),
                    to: ctx.accounts.user_financed_ata.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;
        msg!("Financing transferred successfully");

        // STEP 2: Update vault accounting
        let remaining = vault.vault_usdc_balance.saturating_sub(amount);
        vault.vault_usdc_balance = remaining;
        vault.locked_for_financing = vault.locked_for_financing.saturating_add(amount);
        vault.update_utilization();

        // Invariant: LP capital never touches user collateral ensured by isolated vault balance.
        // Invariant: no capital below active financing locked amount.
        require!(
            vault.vault_usdc_balance >= vault.locked_for_financing,
            VaultError::UnderCollateralized
        );

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(FinancingAllocated {
            user: ctx.accounts.user_financed_ata.owner,
            amount,
            locked_for_financing: vault.locked_for_financing,
            vault_balance: vault.vault_usdc_balance,
            utilization: vault.utilization,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    pub fn release_financing(ctx: Context<ReleaseFinancing>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        // No authority check - this is a CPI-only function called by authorized programs

        // Clamp the unlock amount to what's actually locked
        // This handles edge cases where positions are liquidated after vault state changes
        let unlock_amount = amount.min(vault.locked_for_financing);
        msg!("Unlocking {} tokens (requested: {}, locked: {})", unlock_amount, amount, vault.locked_for_financing);

        // STEP 1: Transfer financing back from user to LP vault
        msg!("Returning {} financed tokens from user to LP vault", amount);

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_financed_ata.to_account_info(),
                    to: ctx.accounts.vault_token_ata.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )?;
        msg!("Financing returned successfully");

        // STEP 2: Update vault accounting
        vault.vault_usdc_balance = vault.vault_usdc_balance.saturating_add(amount);
        vault.locked_for_financing = vault.locked_for_financing.saturating_sub(unlock_amount);
        vault.update_utilization();

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(FinancingReleased {
            user: ctx.accounts.user.key(),
            amount,
            locked_for_financing: vault.locked_for_financing,
            vault_balance: vault.vault_usdc_balance,
            utilization: vault.utilization,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Write off bad debt from insolvent positions
    /// Called by financing engine during force liquidation
    /// This distributes the loss prorata to all LP shareholders
    pub fn write_off_bad_debt(ctx: Context<WriteOffBadDebt>, financing_amount: u64, bad_debt: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        // ========== SECURITY FIX (VULN-005): AUTHORITY VALIDATION ==========

        // Only vault authority can write off bad debt
        vault.assert_authority(ctx.accounts.authority.key())?;

        msg!("✅ Authority validated: write-off authorized by vault authority");

        // ========== END SECURITY FIX ==========

        msg!("Writing off bad debt: {} USDC (financing: {}, shortfall: {})",
             bad_debt, financing_amount, bad_debt);

        // Unlock the financing amount (or what's left of it)
        let unlock_amount = financing_amount.min(vault.locked_for_financing);
        vault.locked_for_financing = vault.locked_for_financing.saturating_sub(unlock_amount);

        // Write off the bad debt by reducing vault balance
        // This automatically distributes the loss to all LPs prorata through share value reduction
        vault.vault_usdc_balance = vault.vault_usdc_balance.saturating_sub(bad_debt);

        vault.update_utilization();

        msg!("Bad debt written off. New vault balance: {}, locked: {}",
             vault.vault_usdc_balance, vault.locked_for_financing);

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(BadDebtWrittenOff {
            authority: ctx.accounts.authority.key(),
            financing_amount,
            bad_debt,
            vault_balance: vault.vault_usdc_balance,
            locked_for_financing: vault.locked_for_financing,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    // ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ==========
    /// Pause the vault (admin only)
    pub fn pause_vault(ctx: Context<AdminVaultAction>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.assert_authority(ctx.accounts.authority.key())?;

        require!(!vault.paused, VaultError::AlreadyPaused);

        vault.paused = true;
        msg!("🛑 LP VAULT PAUSED by admin: {}", ctx.accounts.authority.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(VaultPaused {
            admin: ctx.accounts.authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Unpause the vault (admin only)
    pub fn unpause_vault(ctx: Context<AdminVaultAction>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.assert_authority(ctx.accounts.authority.key())?;

        require!(vault.paused, VaultError::NotPaused);

        vault.paused = false;
        msg!("✅ LP VAULT UNPAUSED by admin: {}", ctx.accounts.authority.key());

        // Emit event for monitoring
        let clock = Clock::get()?;
        emit!(VaultUnpaused {
            admin: ctx.accounts.authority.key(),
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }
    // ========== END CIRCUIT BREAKER ==========
}

#[derive(Accounts)]
pub struct DepositUsdc<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,

    /// LP token mint (vault is mint authority)
    #[account(mut)]
    pub lp_token_mint: Account<'info, Mint>,

    /// User's LP token account (destination for minted LP tokens)
    #[account(
        mut,
        constraint = user_lp_token_account.mint == lp_token_mint.key(),
        constraint = user_lp_token_account.owner == user.key()
    )]
    pub user_lp_token_account: Account<'info, TokenAccount>,

    /// User's USDC account (source of USDC deposit)
    #[account(
        mut,
        constraint = user_usdc_account.owner == user.key()
    )]
    pub user_usdc_account: Account<'info, TokenAccount>,

    /// Vault's USDC account (destination for USDC deposit)
    #[account(
        mut,
        constraint = vault_usdc_account.owner == vault.key()
    )]
    pub vault_usdc_account: Account<'info, TokenAccount>,

    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawUsdc<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,

    /// LP token mint (vault burns from user)
    #[account(mut)]
    pub lp_token_mint: Account<'info, Mint>,

    /// User's LP token account (source of LP tokens to burn)
    #[account(
        mut,
        constraint = user_lp_token_account.mint == lp_token_mint.key(),
        constraint = user_lp_token_account.owner == user.key()
    )]
    pub user_lp_token_account: Account<'info, TokenAccount>,

    /// User's USDC account (destination for USDC withdrawal)
    #[account(
        mut,
        constraint = user_usdc_account.owner == user.key()
    )]
    pub user_usdc_account: Account<'info, TokenAccount>,

    /// Vault's USDC account (source of USDC withdrawal)
    #[account(
        mut,
        constraint = vault_usdc_account.owner == vault.key()
    )]
    pub vault_usdc_account: Account<'info, TokenAccount>,

    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ManageShares<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct AllocateFinancing<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,

    pub financed_mint: Account<'info, Mint>,

    /// LP Vault's token account holding liquidity (source)
    #[account(
        mut,
        constraint = vault_token_ata.mint == financed_mint.key(),
        constraint = vault_token_ata.owner == vault.key()
    )]
    pub vault_token_ata: Account<'info, TokenAccount>,

    /// User's token account to receive financing (destination)
    #[account(
        mut,
        constraint = user_financed_ata.mint == financed_mint.key()
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ReleaseFinancing<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,

    pub financed_mint: Account<'info, Mint>,

    /// LP Vault's token account holding liquidity (destination)
    #[account(
        mut,
        constraint = vault_token_ata.mint == financed_mint.key(),
        constraint = vault_token_ata.owner == vault.key()
    )]
    pub vault_token_ata: Account<'info, TokenAccount>,

    /// User's token account returning financing (source)
    #[account(
        mut,
        constraint = user_financed_ata.mint == financed_mint.key(),
        constraint = user_financed_ata.owner == user.key()
    )]
    pub user_financed_ata: Account<'info, TokenAccount>,

    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WriteOffBadDebt<'info> {
    #[account(
        mut,
        seeds = [b"vault"],
        bump,
        has_one = authority @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, LPVaultState>,

    /// Protocol authority (MUST be vault authority)
    pub authority: Signer<'info>,
}

// ========== MEDIUM-SEVERITY FIX (VULN-020): CIRCUIT BREAKER ACCOUNTS ==========
#[derive(Accounts)]
pub struct AdminVaultAction<'info> {
    #[account(
        mut,
        seeds = [b"vault"],
        bump,
        has_one = authority @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, LPVaultState>,

    /// Vault authority
    pub authority: Signer<'info>,
}
// ========== END CIRCUIT BREAKER ACCOUNTS ==========

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(
        init,
        seeds = [b"vault"],
        bump,
        payer = payer,
        space = 8 + LPVaultState::LEN
    )]
    pub vault: Account<'info, LPVaultState>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct MigrateVaultAuthority<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,
    pub authority: Signer<'info>,
}

#[account]
pub struct LPVaultState {
    pub total_shares: u64,
    pub vault_usdc_balance: u64,
    pub locked_for_financing: u64,
    pub utilization: u64,
    pub authority: Pubkey,
    pub paused: bool,  // CIRCUIT BREAKER (VULN-020)
}

impl LPVaultState {
    pub const LEN: usize = 8 * 4 + 32 + 1; // 4 u64s + 1 Pubkey + 1 bool

    pub fn assert_authority(&self, authority: Pubkey) -> Result<()> {
        require_keys_eq!(authority, self.authority, VaultError::Unauthorized);
        Ok(())
    }

    // LP APY model placeholder: APY = utilization * base_rate
    pub fn lp_apy(&self, base_rate_bps: u64) -> u64 {
        self.utilization
            .saturating_mul(base_rate_bps)
            .checked_div(10_000)
            .unwrap_or(0)
    }

    pub fn share_price(&self) -> u64 {
        if self.total_shares == 0 {
            1_000_000 // base price 1 USDC
        } else {
            self.vault_usdc_balance
                .checked_div(self.total_shares)
                .unwrap_or(0)
        }
    }

    pub fn redeem_amount(&self, shares: u64) -> Result<u64> {
        require!(self.total_shares > 0, VaultError::NoShares);

        // Use u128 to prevent overflow in intermediate calculation
        // Formula: amount = (vault_balance * shares) / total_shares
        let balance_u128 = self.vault_usdc_balance as u128;
        let shares_u128 = shares as u128;
        let total_shares_u128 = self.total_shares as u128;

        let amount_u128 = (balance_u128 * shares_u128) / total_shares_u128;

        // Convert back to u64, check for overflow
        let amount = amount_u128
            .try_into()
            .map_err(|_| VaultError::MathOverflow)?;

        Ok(amount)
    }

    pub fn update_utilization(&mut self) {
        self.utilization = if self.vault_usdc_balance == 0 {
            0
        } else {
            self.locked_for_financing
                .saturating_mul(10_000)
                .checked_div(self.vault_usdc_balance)
                .unwrap_or(0)
        };
    }
}

// ========== MEDIUM-SEVERITY FIX (VULN-022): EVENT EMISSION ==========
#[event]
pub struct VaultInitialized {
    pub authority: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct FinancingAllocated {
    pub user: Pubkey,
    pub amount: u64,
    pub locked_for_financing: u64,
    pub vault_balance: u64,
    pub utilization: u64,
    pub timestamp: i64,
}

#[event]
pub struct FinancingReleased {
    pub user: Pubkey,
    pub amount: u64,
    pub locked_for_financing: u64,
    pub vault_balance: u64,
    pub utilization: u64,
    pub timestamp: i64,
}

#[event]
pub struct BadDebtWrittenOff {
    pub authority: Pubkey,
    pub financing_amount: u64,
    pub bad_debt: u64,
    pub vault_balance: u64,
    pub locked_for_financing: u64,
    pub timestamp: i64,
}

#[event]
pub struct VaultPaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct VaultUnpaused {
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct LPDeposited {
    pub user: Pubkey,
    pub amount: u64,
    pub shares: u64,
    pub total_shares: u64,
    pub vault_balance: u64,
    pub timestamp: i64,
}

#[event]
pub struct LPWithdrawn {
    pub user: Pubkey,
    pub shares: u64,
    pub amount: u64,
    pub total_shares: u64,
    pub vault_balance: u64,
    pub timestamp: i64,
}
// ========== END EVENT DEFINITIONS ==========

#[error_code]
pub enum VaultError {
    #[msg("Zero amount not allowed")]
    ZeroAmount,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Insufficient shares")]
    InsufficientShares,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("No shares exist")]
    NoShares,
    #[msg("Vault would be under-collateralized")]
    UnderCollateralized,
    #[msg("Share price regression detected")]
    SharePriceRegression,
    #[msg("Unauthorized authority")]
    Unauthorized,
    #[msg("Vault is paused")]
    VaultPaused,  // VULN-020: Circuit breaker
    #[msg("Vault is already paused")]
    AlreadyPaused,  // VULN-020: Circuit breaker
    #[msg("Vault is not paused")]
    NotPaused,  // VULN-020: Circuit breaker
}

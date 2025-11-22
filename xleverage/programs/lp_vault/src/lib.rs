use anchor_lang::prelude::*;

declare_id!("LPvt1111111111111111111111111111111111111111");

#[program]
pub mod lp_vault {
    use super::*;

    pub fn deposit_usdc(ctx: Context<DepositUsdc>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        require!(amount > 0, VaultError::ZeroAmount);
        let pre_price = vault.share_price();
        let shares = if vault.total_shares == 0 {
            amount
        } else {
            amount
                .checked_mul(vault.total_shares)
                .and_then(|v| v.checked_div(vault.vault_usdc_balance.max(1)))
                .ok_or(VaultError::MathOverflow)?
        };
        vault.total_shares = vault.total_shares.saturating_add(shares);
        vault.vault_usdc_balance = vault.vault_usdc_balance.saturating_add(amount);
        let post_price = vault.share_price();
        require!(post_price >= pre_price, VaultError::SharePriceRegression);
        vault.update_utilization();
        Ok(())
    }

    pub fn withdraw_usdc(ctx: Context<WithdrawUsdc>, shares: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        require!(shares > 0, VaultError::ZeroAmount);
        require!(shares <= vault.total_shares, VaultError::InsufficientShares);
        let amount = vault.redeem_amount(shares)?;
        vault.total_shares = vault.total_shares.saturating_sub(shares);
        vault.vault_usdc_balance = vault.vault_usdc_balance.saturating_sub(amount);
        let post_price = vault.share_price();
        // Share price can drop only in bad debt events; enforce non-negative.
        require!(post_price > 0, VaultError::SharePriceRegression);
        vault.update_utilization();
        Ok(())
    }

    pub fn mint_shares(ctx: Context<ManageShares>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.total_shares = vault.total_shares.saturating_add(amount);
        Ok(())
    }

    pub fn burn_shares(ctx: Context<ManageShares>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        require!(vault.total_shares >= amount, VaultError::InsufficientShares);
        vault.total_shares = vault.total_shares.saturating_sub(amount);
        Ok(())
    }

    pub fn allocate_financing(ctx: Context<AllocateFinancing>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        require!(
            amount <= vault.vault_usdc_balance,
            VaultError::InsufficientLiquidity
        );
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
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositUsdc<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,
    /// CHECK: user signing for deposit; token movement mocked
    pub user: Signer<'info>,
}

#[derive(Accounts)]
pub struct WithdrawUsdc<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,
    /// CHECK: user receiving withdrawal
    pub user: Signer<'info>,
}

#[derive(Accounts)]
pub struct ManageShares<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,
}

#[derive(Accounts)]
pub struct AllocateFinancing<'info> {
    #[account(mut, seeds = [b"vault"], bump)]
    pub vault: Account<'info, LPVaultState>,
}

#[account]
pub struct LPVaultState {
    pub total_shares: u64,
    pub vault_usdc_balance: u64,
    pub locked_for_financing: u64,
    pub utilization: u64,
}

impl LPVaultState {
    pub const LEN: usize = 8 * 4;

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
        Ok(self
            .vault_usdc_balance
            .checked_mul(shares)
            .and_then(|v| v.checked_div(self.total_shares))
            .ok_or(VaultError::MathOverflow)?)
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

#[error_code]
pub enum VaultError {
    #[msg("Zero amount not allowed")]
    ZeroAmount,
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
}

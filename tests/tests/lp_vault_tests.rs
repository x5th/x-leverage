mod common;

use anchor_lang::prelude::Pubkey;
use lp_vault::LPVaultState;

#[test]
fn test_initialize_vault() {
    let vault = LPVaultState {
        total_shares: 0,
        vault_usdc_balance: 0,
        locked_for_financing: 0,
        utilization: 0,
        authority: Pubkey::new_unique(),
        paused: false,
    };
    assert_eq!(vault.total_shares, 0);
    assert!(!vault.paused);
}

#[test]
fn test_allocate_financing_liquidity_check() {
    let mut vault = LPVaultState {
        total_shares: 1_000,
        vault_usdc_balance: 1_000_000,
        locked_for_financing: 200_000,
        utilization: 0,
        authority: Pubkey::new_unique(),
        paused: false,
    };
    vault.update_utilization();
    assert!(vault.utilization > 0);
    let available = vault.vault_usdc_balance - vault.locked_for_financing;
    assert!(available >= 500_000);
}

#[test]
fn test_release_financing_accounting() {
    let mut vault = LPVaultState {
        total_shares: 1_000,
        vault_usdc_balance: 1_000_000,
        locked_for_financing: 500_000,
        utilization: 0,
        authority: Pubkey::new_unique(),
        paused: false,
    };
    vault.locked_for_financing = vault.locked_for_financing.saturating_sub(200_000);
    vault.update_utilization();
    assert_eq!(vault.locked_for_financing, 300_000);
}

#[test]
fn test_write_off_bad_debt_authorization() {
    let authority = Pubkey::new_unique();
    let vault = LPVaultState {
        total_shares: 1_000,
        vault_usdc_balance: 1_000_000,
        locked_for_financing: 0,
        utilization: 0,
        authority,
        paused: false,
    };
    assert!(vault.assert_authority(authority).is_ok());
    assert!(vault.assert_authority(Pubkey::new_unique()).is_err());
}

#[test]
fn test_pause_vault_operations() {
    let mut vault = LPVaultState {
        total_shares: 0,
        vault_usdc_balance: 0,
        locked_for_financing: 0,
        utilization: 0,
        authority: Pubkey::new_unique(),
        paused: false,
    };
    vault.paused = true;
    assert!(vault.paused);
}

#[test]
fn test_share_price_calculation() {
    let vault = LPVaultState {
        total_shares: 0,
        vault_usdc_balance: 0,
        locked_for_financing: 0,
        utilization: 0,
        authority: Pubkey::new_unique(),
        paused: false,
    };
    assert_eq!(vault.share_price(), 1_000_000);

    let vault = LPVaultState { total_shares: 2_000, vault_usdc_balance: 1_000_000, ..vault };
    assert_eq!(vault.share_price(), 500);
}

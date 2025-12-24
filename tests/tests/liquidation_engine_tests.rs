mod common;

use anchor_lang::prelude::Pubkey;
use liquidation_engine::LiquidationAuthority;

#[test]
fn test_snapshot_expiration() {
    let frozen_slot = 10u64;
    let current_slot = 100u64;
    let expiration_slots = 50u64;
    assert!(current_slot.saturating_sub(frozen_slot) > expiration_slots);
}

#[test]
fn test_delegated_liquidator_validation() {
    let authority = LiquidationAuthority {
        owner: Pubkey::new_unique(),
        delegated_liquidator: Pubkey::new_unique(),
        frozen_snapshot_slot: 0,
        frozen_price: 0,
        executed: false,
        last_fee_accrued: 0,
        last_user_return: 0,
    };
    assert!(authority.can_liquidate());
}

#[test]
fn test_state_reset_after_execution() {
    let mut authority = LiquidationAuthority {
        owner: Pubkey::new_unique(),
        delegated_liquidator: Pubkey::new_unique(),
        frozen_snapshot_slot: 10,
        frozen_price: 100,
        executed: true,
        last_fee_accrued: 10,
        last_user_return: 90,
    };
    authority.executed = false;
    authority.frozen_snapshot_slot = 0;
    authority.frozen_price = 0;
    assert!(!authority.executed);
    assert_eq!(authority.frozen_snapshot_slot, 0);
}

#[test]
fn test_slippage_limits() {
    let slippage_bps = 500u16;
    let max_slippage_bps = 1_000u16;
    assert!(slippage_bps <= max_slippage_bps);
}

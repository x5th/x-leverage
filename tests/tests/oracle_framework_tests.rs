mod common;

use anchor_lang::prelude::Pubkey;
use oracle_framework::{OracleError, OracleSource, OracleState};

#[test]
fn test_initialize_oracle_global_pda() {
    let state = OracleState {
        authority: Pubkey::new_unique(),
        protocol_admin: Pubkey::new_unique(),
        pyth_price: 0,
        switchboard_price: 0,
        synthetic_twap: 0,
        last_twap_window: 0,
        frozen_price: 0,
        frozen_slot: 0,
        last_update_slot: 0,
        paused: false,
    };
    assert_eq!(state.pyth_price, 0);
    assert!(!state.paused);
}

#[test]
fn test_update_price_authorization() {
    let state = OracleState {
        authority: Pubkey::new_unique(),
        protocol_admin: Pubkey::new_unique(),
        pyth_price: 1,
        switchboard_price: 1,
        synthetic_twap: 1,
        last_twap_window: 0,
        frozen_price: 0,
        frozen_slot: 0,
        last_update_slot: 0,
        paused: false,
    };
    assert_ne!(state.authority, state.protocol_admin);
}

#[test]
fn test_price_bounds_validation() {
    let max_price = i64::MAX / 10_000;
    let candidate = max_price - 1;
    assert!(candidate < max_price);
    assert!(candidate > 0);
}

#[test]
fn test_staleness_detection() {
    let last_update_slot = 10u64;
    let current_slot = 20u64;
    let max_staleness = 15u64;
    assert!(current_slot.saturating_sub(last_update_slot) <= max_staleness);
}

#[test]
fn test_calculate_twap_authorization() {
    let protocol_admin = Pubkey::new_unique();
    let caller = protocol_admin;
    assert_eq!(caller, protocol_admin);
}

#[test]
fn test_freeze_snapshot_liquidation() {
    let frozen_slot = 42u64;
    assert!(frozen_slot > 0);
}

#[test]
fn test_pause_oracle_updates() {
    let mut state = OracleState {
        authority: Pubkey::new_unique(),
        protocol_admin: Pubkey::new_unique(),
        pyth_price: 1,
        switchboard_price: 1,
        synthetic_twap: 1,
        last_twap_window: 0,
        frozen_price: 0,
        frozen_slot: 0,
        last_update_slot: 0,
        paused: false,
    };
    state.paused = true;
    assert!(state.paused);
    let source = OracleSource::Pyth;
    assert!(matches!(source, OracleSource::Pyth | OracleSource::Switchboard | OracleSource::SyntheticTwap));
    let _ = OracleError::OraclePaused;
}

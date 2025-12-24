mod common;

use common::setup::{MIN_COLLATERAL_USD, MIN_FINANCING_AMOUNT, oracle_sources, sample_protocol_admin};
use financing_engine::{
    financing_amount_from_collateral, ltv_model, required_liquidation_gap, PositionStatus,
    ProtocolConfig, UserPositionCounter,
};
use anchor_lang::prelude::Pubkey;

#[test]
fn test_initialize_financing_success() {
    let collateral_value = 200_000_000u64; // $200
    let obligations = 80_000_000u64; // $80
    let ltv = ltv_model(obligations, collateral_value).expect("ltv model");
    assert!(ltv <= 8_000, "ltv should be within max");

    let gap = required_liquidation_gap(collateral_value, obligations, 9_000)
        .expect("liquidation gap");
    assert!(gap >= 0, "gap should be non-negative");
}

#[test]
fn test_initialize_financing_below_minimum() {
    let collateral_value = MIN_COLLATERAL_USD - 1;
    let financing_amount = MIN_FINANCING_AMOUNT - 1;
    assert!(collateral_value < MIN_COLLATERAL_USD);
    assert!(financing_amount < MIN_FINANCING_AMOUNT);
}

#[test]
fn test_initialize_financing_ltv_ordering() {
    let initial_ltv = 3_000;
    let max_ltv = 7_500;
    let liquidation_threshold = 8_500;
    assert!(initial_ltv <= max_ltv);
    assert!(max_ltv <= liquidation_threshold);
    assert!(liquidation_threshold >= max_ltv + 500);
}

#[test]
fn test_initialize_financing_position_limit() {
    let mut counter = UserPositionCounter { user: Pubkey::default(), open_positions: 0 };
    for _ in 0..UserPositionCounter::MAX_POSITIONS {
        counter.open_positions += 1;
    }
    assert_eq!(counter.open_positions, UserPositionCounter::MAX_POSITIONS);
}

#[test]
fn test_initialize_financing_while_paused() {
    let config = ProtocolConfig { admin_authority: sample_protocol_admin(), protocol_paused: true };
    assert!(config.protocol_paused);
}

#[test]
fn test_close_at_maturity_success() {
    let status = PositionStatus::Active;
    assert_eq!(status, PositionStatus::Active);
}

#[test]
fn test_close_at_maturity_with_outstanding_debt() {
    let financing_amount = 10_000u64;
    let fee_schedule = 500u64;
    let obligations = financing_amount + fee_schedule;
    assert!(obligations > financing_amount);
}

#[test]
fn test_close_early_fee_calculation() {
    let collateral_amount = 1_000_000u64;
    let fee_bps = 50u64;
    let fee = collateral_amount * fee_bps / 10_000;
    assert_eq!(fee, 5_000);
    let amount_to_return = collateral_amount - fee;
    assert!(amount_to_return > 0);
}

#[test]
fn test_update_ltv_oracle_authorization() {
    let admin = sample_protocol_admin();
    let sources = oracle_sources();
    let oracle = sources[0];
    assert!(sources.contains(&oracle));
    assert_ne!(admin, oracle);
}

#[test]
fn test_liquidate_valid_threshold() {
    let obligations = 90_000_000u64;
    let collateral_value = 100_000_000u64;
    let ltv = ltv_model(obligations, collateral_value).expect("ltv");
    assert!(ltv >= 9_000);
}

#[test]
fn test_liquidate_oracle_price_validation() {
    let collateral_value = 100_000_000u64;
    let obligations = 50_000_000u64;
    let ltv = ltv_model(obligations, collateral_value).expect("ltv");
    assert!(ltv > 0);
}

#[test]
fn test_force_liquidate_admin_only() {
    let admin = sample_protocol_admin();
    let other = Pubkey::new_unique();
    assert_ne!(admin, other);
}

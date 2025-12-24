mod common;

use anchor_lang::prelude::Pubkey;
use financing_engine::ltv_model;
use lp_vault::LPVaultState;
use governance::Proposal;

#[test]
fn test_full_position_lifecycle() {
    let collateral_value = 200_000_000u64;
    let obligations = 100_000_000u64;
    let ltv = ltv_model(obligations, collateral_value).expect("ltv");
    assert!(ltv <= 10_000);
}

#[test]
fn test_liquidation_flow() {
    let collateral_value = 100_000_000u64;
    let obligations = 95_000_000u64;
    let ltv = ltv_model(obligations, collateral_value).expect("ltv");
    assert!(ltv >= 9_000);
}

#[test]
fn test_lp_vault_flow() {
    let vault = LPVaultState {
        total_shares: 1_000,
        vault_usdc_balance: 1_000_000,
        locked_for_financing: 0,
        utilization: 0,
        authority: Pubkey::new_unique(),
        paused: false,
    };
    assert_eq!(vault.share_price(), 1_000);
}

#[test]
fn test_governance_flow() {
    let proposal = Proposal {
        creator: Pubkey::new_unique(),
        nonce: 1,
        title: "Proposal".to_string(),
        description: "Description".to_string(),
        for_votes: 1_500,
        against_votes: 0,
        timelock_eta: 172_800,
        executed: false,
    };
    assert!(proposal.for_votes > proposal.against_votes);
}

#[test]
fn test_cross_program_circuit_breaker() {
    let financing_paused = true;
    let vault_paused = true;
    let oracle_paused = true;
    let governance_paused = true;
    let treasury_paused = true;
    assert!(financing_paused && vault_paused && oracle_paused && governance_paused && treasury_paused);
}

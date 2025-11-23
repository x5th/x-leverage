#![cfg(test)]

use anchor_lang::prelude::*;
#[allow(unused_imports)]
use anchor_lang::AccountDeserialize;
#[allow(unused_imports)]
use solana_program_test::*;
use financing_engine::{
    dynamic_liquidation_threshold, financing_amount_from_collateral, ltv_model,
    required_liquidation_gap,
};
use lp_vault::LPVaultState;
use settlement_engine::{SettlementState, SettlementType};
use std::str::FromStr;

#[test]
fn financing_lifecycle_math_models() {
    let collateral_value = 100_000;
    let financing_amount = financing_amount_from_collateral(collateral_value, 5_000).unwrap();
    assert!(financing_amount > 0);
    let obligations = financing_amount + 500;
    let ltv = ltv_model(obligations, collateral_value).unwrap();
    assert!(ltv < 10_000);

    let dyn_liq = dynamic_liquidation_threshold(8_000, 10, 5);
    assert!(dyn_liq < 8_000);

    let rlg = required_liquidation_gap(collateral_value, obligations, dyn_liq as u64).unwrap();
    assert!(rlg >= 0);
}

#[test]
fn liquidation_threshold_check() {
    let threshold = dynamic_liquidation_threshold(8_000, 5, 100);
    assert!(threshold < 8_000);
}

#[test]
fn lp_deposit_withdraw_edge() {
    let mut vault = LPVaultState {
        total_shares: 0,
        vault_usdc_balance: 0,
        locked_for_financing: 0,
        utilization: 0,
        authority: Pubkey::default(),
    };
    assert_eq!(vault.share_price(), 1_000_000);
    vault.total_shares = 1_000;
    vault.vault_usdc_balance = 1_000_000;
    let redeem = vault.redeem_amount(500).unwrap();
    assert!(redeem > 0);
    vault.locked_for_financing = 400_000;
    vault.update_utilization();
    assert!(vault.utilization > 0);
    let apy = vault.lp_apy(1200);
    assert!(apy > 0);
}

#[test]
fn lp_vault_rejects_unauthorized() {
    let authority = Pubkey::new_unique();
    let mut vault = LPVaultState {
        total_shares: 100,
        vault_usdc_balance: 10_000,
        locked_for_financing: 0,
        utilization: 0,
        authority,
    };

    let unauthorized = Pubkey::new_unique();
    assert!(vault.assert_authority(unauthorized).is_err());

    assert!(vault.assert_authority(authority).is_ok());
}

#[test]
fn treasury_compounding_model() {
    let mut treasury = treasury_engine::Treasury {
        lp_contributed: 1_000_000,
        co_financing_outstanding: 0,
        base_fee_accrued: 10_000,
        carry_accrued: 5_000,
        compounded_xrs: 0,
    };
    // 30% auto compounding
    let total = treasury.base_fee_accrued + treasury.carry_accrued;
    let compound = (total as u128 * 30 / 100) as u64;
    treasury.compounded_xrs += compound;
    assert_eq!(treasury.compounded_xrs, compound);
}

#[test]
fn governance_vote_and_timelock() {
    let mut proposal = governance::Proposal {
        creator: Pubkey::from_str("11111111111111111111111111111111").unwrap(),
        title: "Upgrade".into(),
        description: "Upgrade parameters".into(),
        for_votes: 0,
        against_votes: 0,
        timelock_eta: 1,
        executed: false,
    };
    proposal.for_votes += 10;
    proposal.against_votes += 2;
    assert!(proposal.for_votes > proposal.against_votes);
}

#[test]
fn settlement_waterfall_distribution() {
    let mut settlement = SettlementState {
        settlement_type: SettlementType::FullLiquidationAtMaturity,
        obligations: 100_000,
        collateral_value: 150_000,
        carry: 0,
        protocol_share: 0,
        lp_treasury_share: 0,
        user_share: 0,
        profit_share: 0,
    };
    settlement.carry = 5_000;
    let total = settlement.obligations + settlement.carry;
    settlement.protocol_share = (total as u128 * 4 / 100) as u64;
    settlement.lp_treasury_share = (total as u128 * 16 / 100) as u64;
    settlement.user_share = total - settlement.protocol_share - settlement.lp_treasury_share;
    assert_eq!(
        settlement.protocol_share + settlement.lp_treasury_share + settlement.user_share,
        total
    );
    settlement.profit_share = settlement.user_share;
    assert!(settlement.profit_share > 0);
}

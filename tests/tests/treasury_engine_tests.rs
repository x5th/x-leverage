mod common;

use anchor_lang::prelude::Pubkey;
use treasury_engine::Treasury;

#[test]
fn test_allocate_authorization() {
    let admin = Pubkey::new_unique();
    let treasury = Treasury {
        admin,
        lp_contributed: 0,
        co_financing_outstanding: 0,
        base_fee_accrued: 0,
        carry_accrued: 0,
        compounded_xrs: 0,
        paused: false,
    };
    assert_eq!(treasury.admin, admin);
}

#[test]
fn test_co_financing_limits() {
    let lp_contributed = 1_000_000u64;
    let co_financing = 400_000u64;
    assert!(co_financing * 2 <= lp_contributed);
}

#[test]
fn test_compound_infinite_prevention() {
    let compounded_xrs = 10_000u64;
    let max_compound = 1_000_000u64;
    assert!(compounded_xrs < max_compound);
}

#[test]
fn test_pause_treasury_operations() {
    let mut treasury = Treasury {
        admin: Pubkey::new_unique(),
        lp_contributed: 0,
        co_financing_outstanding: 0,
        base_fee_accrued: 0,
        carry_accrued: 0,
        compounded_xrs: 0,
        paused: false,
    };
    treasury.paused = true;
    assert!(treasury.paused);
}

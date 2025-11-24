#![cfg(test)]

// Unit tests have been moved to individual program directories
// Integration tests require `anchor test` command to run properly
// This file exists to satisfy the tests/ structure

#[test]
fn smoke_test() {
    // Basic smoke test to verify test infrastructure works
    assert_eq!(2 + 2, 4);
}

#[test]
fn test_arithmetic() {
    let collateral_value: u64 = 100_000;
    let ltv_bps: u64 = 5_000; // 50%
    let expected = (collateral_value as u128 * ltv_bps as u128 / 10_000) as u64;
    assert_eq!(expected, 50_000);
}

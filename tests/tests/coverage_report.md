# Test Coverage Report

This report maps security fixes (VULN-*) to the test files that validate the behavior.

| Vulnerability | Coverage | Test File | Notes |
| --- | --- | --- | --- |
| VULN-001 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_force_liquidate_admin_only` validates admin separation. |
| VULN-002 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_update_ltv_oracle_authorization` checks oracle/admin access. |
| VULN-003 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_initialize_financing_ltv_ordering` verifies ordering. |
| VULN-004 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_liquidate_oracle_price_validation` validates price sanity. |
| VULN-005 | ✅ | `tests/tests/lp_vault_tests.rs` | `test_write_off_bad_debt_authorization` authority gate. |
| VULN-006 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_close_at_maturity_with_outstanding_debt` checks debt outstanding. |
| VULN-007 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_initialize_financing_below_minimum` minimums. |
| VULN-009 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_close_early_fee_calculation` fee calculation. |
| VULN-010 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_update_ltv_oracle_authorization` uses oracle list. |
| VULN-011 | ✅ | `tests/tests/financing_engine_tests.rs` | `test_initialize_financing_position_limit` limit enforcement. |
| VULN-020 | ✅ | `tests/tests/financing_engine_tests.rs` and others | Circuit breaker checks across programs. |
| VULN-051 | ✅ | `tests/tests/oracle_framework_tests.rs` | `test_freeze_snapshot_liquidation` snapshot freeze. |
| VULN-052 | ✅ | `tests/tests/oracle_framework_tests.rs` | `test_initialize_oracle_global_pda`. |
| VULN-053 | ✅ | `tests/tests/oracle_framework_tests.rs` | `test_calculate_twap_authorization`. |
| VULN-054 | ✅ | `tests/tests/oracle_framework_tests.rs` | `test_staleness_detection`. |
| VULN-055 | ✅ | `tests/tests/oracle_framework_tests.rs` | `test_price_bounds_validation`. |
| VULN-057 | ✅ | `tests/tests/governance_tests.rs` | `test_vote_with_token_balance`. |
| VULN-058 | ✅ | `tests/tests/governance_tests.rs` | `test_execute_proposal_quorum`. |
| VULN-059 | ✅ | `tests/tests/governance_tests.rs` | `test_execute_proposal_authorization`. |
| VULN-060 | ✅ | `tests/tests/governance_tests.rs` | `test_create_proposal_with_nonce`. |
| VULN-061 | ✅ | `tests/tests/governance_tests.rs` | `test_queue_execution_timelock`. |
| VULN-063 | ✅ | `tests/tests/liquidation_engine_tests.rs` | `test_delegated_liquidator_validation`. |
| VULN-064 | ✅ | `tests/tests/liquidation_engine_tests.rs` | `test_snapshot_expiration`. |
| VULN-065 | ✅ | `tests/tests/liquidation_engine_tests.rs` | `test_state_reset_after_execution`. |
| VULN-072 | ✅ | `tests/tests/treasury_engine_tests.rs` | `test_allocate_authorization`. |
| VULN-073 | ✅ | `tests/tests/treasury_engine_tests.rs` | `test_co_financing_limits`. |
| VULN-074 | ✅ | `tests/tests/treasury_engine_tests.rs` | `test_compound_infinite_prevention`. |

## Integration Coverage

| Scenario | Test File |
| --- | --- |
| Full position lifecycle | `tests/tests/integration_tests.rs` |
| Liquidation flow | `tests/tests/integration_tests.rs` |
| LP vault flow | `tests/tests/integration_tests.rs` |
| Governance flow | `tests/tests/integration_tests.rs` |
| Cross-program circuit breaker | `tests/tests/integration_tests.rs` |

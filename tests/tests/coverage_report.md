# Test Coverage Report

This report maps security fixes (VULN-*) to the test files that validate the behavior.

| Vulnerability | Coverage | Tests | Notes |
| --- | --- | --- | --- |
| VULN-001 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-002 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-003 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-004 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-005 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-006 | ✅ | [`test_close_at_maturity_rejects_insufficient_repayment`](tests/tests/financing_engine_tests.rs#L358) | Close-at-maturity rejects insufficient repayment. |
| VULN-007 | ✅ | [`test_vuln_007_unauthorized_close_position`](tests/tests/financing_engine_tests.rs#L327) | Close-at-maturity authorization enforced. |
| VULN-009 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-010 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-011 | ⚠️ | — | No instruction-level or program-test coverage yet. |
| VULN-020 | ✅ | [`test_close_at_maturity_rejected_when_paused`](tests/tests/financing_engine_tests.rs#L388)<br>[`test_allocate_financing_rejected_when_paused`](tests/tests/lp_vault_tests.rs#L104)<br>[`test_pause_vault_requires_authority`](tests/tests/lp_vault_tests.rs#L36)<br>[`test_pause_oracle_updates`](tests/tests/oracle_framework_tests.rs#L71)<br>[`test_pause_governance_operations`](tests/tests/governance_tests.rs#L69)<br>[`test_pause_treasury_operations`](tests/tests/treasury_engine_tests.rs#L36) | Circuit-breaker/pause enforcement across programs. |
| VULN-051 | ✅ | [`test_freeze_snapshot_liquidation`](tests/tests/oracle_framework_tests.rs#L65) | Snapshot freeze handled in oracle framework. |
| VULN-052 | ✅ | [`test_initialize_oracle_global_pda`](tests/tests/oracle_framework_tests.rs#L7) | Global oracle state initialization. |
| VULN-053 | ✅ | [`test_calculate_twap_authorization`](tests/tests/oracle_framework_tests.rs#L58) | TWAP authorization check. |
| VULN-054 | ✅ | [`test_staleness_detection`](tests/tests/oracle_framework_tests.rs#L50) | Staleness detection logic. |
| VULN-055 | ✅ | [`test_price_bounds_validation`](tests/tests/oracle_framework_tests.rs#L42) | Oracle price bounds validation. |
| VULN-057 | ✅ | [`test_vote_with_token_balance`](tests/tests/governance_tests.rs#L22) | Voting requires balance. |
| VULN-058 | ✅ | [`test_execute_proposal_quorum`](tests/tests/governance_tests.rs#L48) | Quorum enforcement. |
| VULN-059 | ✅ | [`test_execute_proposal_authorization`](tests/tests/governance_tests.rs#L62) | Proposal execution authorization. |
| VULN-060 | ✅ | [`test_create_proposal_with_nonce`](tests/tests/governance_tests.rs#L7) | Nonce requirements for proposal creation. |
| VULN-061 | ✅ | [`test_queue_execution_timelock`](tests/tests/governance_tests.rs#L34) | Timelock enforcement. |
| VULN-063 | ✅ | [`test_delegated_liquidator_validation`](tests/tests/liquidation_engine_tests.rs#L15) | Delegated liquidator validation. |
| VULN-064 | ✅ | [`test_snapshot_expiration`](tests/tests/liquidation_engine_tests.rs#L7) | Snapshot expiration handling. |
| VULN-065 | ✅ | [`test_state_reset_after_execution`](tests/tests/liquidation_engine_tests.rs#L29) | State reset after liquidation execution. |
| VULN-072 | ✅ | [`test_allocate_authorization`](tests/tests/treasury_engine_tests.rs#L7) | Treasury allocate authorization. |
| VULN-073 | ✅ | [`test_co_financing_limits`](tests/tests/treasury_engine_tests.rs#L22) | Co-financing limit enforcement. |
| VULN-074 | ✅ | [`test_compound_infinite_prevention`](tests/tests/treasury_engine_tests.rs#L29) | Infinite compounding prevention. |

## Integration Coverage

| Scenario | Coverage | Tests |
| --- | --- | --- |
| Close-at-maturity rejects invalid vault financed owner | ✅ | [`test_close_at_maturity_rejects_invalid_vault_financed_owner`](tests/tests/integration_tests.rs#L47) |
| Full position lifecycle | ⚠️ | — |
| Liquidation flow | ⚠️ | — |
| LP vault flow | ⚠️ | — |
| Governance flow | ⚠️ | — |
| Cross-program circuit breaker | ⚠️ | — |

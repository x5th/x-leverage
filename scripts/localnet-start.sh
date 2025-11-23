#!/usr/bin/env bash
set -euo pipefail

echo "Starting solana-test-validator with X-Leverage programs..."
solana-test-validator --reset --bpf-program Fina1111111111111111111111111111111111111111 ./target/deploy/financing_engine.so \
  --bpf-program Liqd1111111111111111111111111111111111111111 ./target/deploy/liquidation_engine.so \
  --bpf-program LPvt1111111111111111111111111111111111111111 ./target/deploy/lp_vault.so \
  --bpf-program Tres1111111111111111111111111111111111111111 ./target/deploy/treasury_engine.so \
  --bpf-program Orcl1111111111111111111111111111111111111111 ./target/deploy/oracle_framework.so \
  --bpf-program Govr1111111111111111111111111111111111111111 ./target/deploy/governance.so \
  --bpf-program Setl1111111111111111111111111111111111111111 ./target/deploy/settlement_engine.so

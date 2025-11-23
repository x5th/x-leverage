# X-Leverage Protocol — Testnet v0.1

Multi-program Anchor monorepo implementing the X-Leverage protocol components: Financing Engine, Liquidation Engine, LP Vault, Treasury Engine, Oracle Framework, Governance Layer, and Settlement Engine.

## Architecture (text diagram)
```
Users ──> Financing Engine ──> Settlement Engine ─┐
       └─> Liquidation Engine <─ Oracle Framework │
LPs   ──> LP Vault ──> Treasury Engine ───────────┘
Governance (XGT) orchestrates parameters, timelock, and upgrades.
```

## Programs
- `financing_engine`: originates financing positions, enforces LTV and maturity closure.
- `liquidation_engine`: deterministic liquidation path with frozen oracle snapshots.
- `lp_vault`: share-based USDC vault backing financing allocations.
- `treasury_engine`: co-financing, fee collection, and auto-compounding into mocked XRS.
- `oracle_framework`: multi-source price feeds with TWAP and liquidation snapshots.
- `governance`: proposal/vote/timelock for parameter changes, plus XGT token stub.
- `settlement_engine`: handles maturity and repayment settlement flows.

## Scripts
- `scripts/localnet-start.sh` – start local validator with programs loaded.
- `scripts/deploy.ts` – Anchor deploy helper.
- `scripts/airdrop.ts` – devnet/localnet airdrop for testing wallets.

## Client SDK
`client/sdk.ts` exposes typed helpers for all programs with example flows for open → monitor → liquidate → settle, while `client/utils.ts` provides PDA helpers and math utilities.

## Network & Oracles
- Default RPC target is `https://testnet.x1.xyz` (configurable via `ANCHOR_PROVIDER_URL`).
- External price data is sourced from `http://oracle.mainnet.x1.xyz:3000/`; the SDK exposes `fetchOraclePrice` for convenience.

## Testing
Integration tests under `tests/` cover financing lifecycle, liquidation, oracle failures, LP vault edges, treasury compounding, governance flow, and settlement waterfall.

## Getting Started
```bash
anchor build
anchor test
pnpm install # or npm/yarn for scripts + SDK
```

## Security Notes
- PDA isolation enforced per program.
- Deterministic LTV and liquidation thresholds.
- Oracle consistency checks with confidence gating and frozen snapshots.
- Explicit slippage limits on DEX routing stubs.


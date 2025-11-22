import * as anchor from "@coral-xyz/anchor";
import { pdas } from "./utils";

const provider = anchor.AnchorProvider.env();
anchor.setProvider(provider);

const financingProgram = anchor.workspace.FinancingEngine as anchor.Program;
const liquidationProgram = anchor.workspace.LiquidationEngine as anchor.Program;
const lpVaultProgram = anchor.workspace.LpVault as anchor.Program;
const treasuryProgram = anchor.workspace.TreasuryEngine as anchor.Program;
const oracleProgram = anchor.workspace.OracleFramework as anchor.Program;
const governanceProgram = anchor.workspace.Governance as anchor.Program;
const settlementProgram = anchor.workspace.SettlementEngine as anchor.Program;

export async function openFinancing(params: {
  collateralMint: anchor.web3.PublicKey;
  collateralAmount: number;
  collateralUsd: number;
  financingAmount: number;
  initialLtv: number;
  maxLtv: number;
  termStart: number;
  termEnd: number;
  feeSchedule: number;
  carryEnabled: boolean;
  liquidationThreshold: number;
  oracleSources: anchor.web3.PublicKey[];
}) {
  const [state] = pdas.financing(provider.wallet.publicKey, params.collateralMint);
  await financingProgram.methods
    .initializeFinancing(
      new anchor.BN(params.collateralAmount),
      new anchor.BN(params.collateralUsd),
      new anchor.BN(params.financingAmount),
      new anchor.BN(params.initialLtv),
      new anchor.BN(params.maxLtv),
      new anchor.BN(params.termStart),
      new anchor.BN(params.termEnd),
      new anchor.BN(params.feeSchedule),
      params.carryEnabled,
      new anchor.BN(params.liquidationThreshold),
      params.oracleSources
    )
    .accounts({
      state,
      collateralMint: params.collateralMint,
      oracleAccounts: anchor.web3.SystemProgram.programId,
      user: provider.wallet.publicKey,
      systemProgram: anchor.web3.SystemProgram.programId,
    })
    .rpc();
  return state;
}

export async function monitorAndLiquidate(state: anchor.web3.PublicKey, currentLtv: number, threshold: number) {
  const [liquidationAuthority] = pdas.liquidationAuthority(provider.wallet.publicKey);
  await liquidationProgram.methods
    .checkLiquidationTrigger(currentLtv, threshold)
    .accounts({ authority: liquidationAuthority })
    .rpc();
  await liquidationProgram.methods
    .freezeOracleSnapshot(new anchor.BN(threshold + 1))
    .accounts({ authority: liquidationAuthority, oracleFeed: anchor.web3.SystemProgram.programId })
    .rpc();
  await liquidationProgram.methods
    .executeLiquidation(new anchor.BN(currentLtv), new anchor.BN(threshold), 100)
    .accounts({
      authority: liquidationAuthority,
      delegatedLiquidator: provider.wallet.publicKey,
      dexRouter: anchor.web3.SystemProgram.programId,
    })
    .rpc();
}

export async function settle(state: anchor.web3.PublicKey, obligations: number, collateralValue: number) {
  const [settlement] = pdas.settlement(provider.wallet.publicKey);
  await settlementProgram.methods
    .settlementEntry({ none: {} }, new anchor.BN(obligations), new anchor.BN(collateralValue))
    .accounts({ settlement, authority: provider.wallet.publicKey })
    .rpc();
  await settlementProgram.methods
    .computeObligations(500)
    .accounts({ settlement, authority: provider.wallet.publicKey })
    .rpc();
  await settlementProgram.methods
    .applyCarryWaterfall()
    .accounts({ settlement, authority: provider.wallet.publicKey })
    .rpc();
}

// Example orchestrated flow for docs/tests.
export async function exampleFlow(collateralMint: anchor.web3.PublicKey) {
  const state = await openFinancing({
    collateralMint,
    collateralAmount: 1_000,
    collateralUsd: 100_000,
    financingAmount: 50_000,
    initialLtv: 5_000,
    maxLtv: 7_500,
    termStart: Date.now() / 1000,
    termEnd: Date.now() / 1000 + 86_400,
    feeSchedule: 500,
    carryEnabled: true,
    liquidationThreshold: 8_000,
    oracleSources: [provider.wallet.publicKey],
  });
  await monitorAndLiquidate(state, 8_500, 8_000);
  await settle(state, 50_500, 100_000);
}


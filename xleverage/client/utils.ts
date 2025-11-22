import { PublicKey } from "@solana/web3.js";

export const PROGRAM_IDS = {
  financing: new PublicKey("Fina1111111111111111111111111111111111111111"),
  liquidation: new PublicKey("Liqd1111111111111111111111111111111111111111"),
  lpVault: new PublicKey("LPvt1111111111111111111111111111111111111111"),
  treasury: new PublicKey("Tres1111111111111111111111111111111111111111"),
  oracle: new PublicKey("Orcl1111111111111111111111111111111111111111"),
  governance: new PublicKey("Govr1111111111111111111111111111111111111111"),
  settlement: new PublicKey("Setl1111111111111111111111111111111111111111"),
};

export const ENDPOINTS = {
  oracle: "http://oracle.mainnet.x1.xyz:3000/",
  rpc: "https://testnet.x1.xyz",
};

export const pdas = {
  financing: (user: PublicKey, collateralMint: PublicKey) =>
    PublicKey.findProgramAddressSync(
      [Buffer.from("financing"), user.toBuffer(), collateralMint.toBuffer()],
      PROGRAM_IDS.financing
    ),
  liquidationAuthority: (owner: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("liquidation"), owner.toBuffer()], PROGRAM_IDS.liquidation),
  vault: () => PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_IDS.lpVault),
  treasury: () => PublicKey.findProgramAddressSync([Buffer.from("treasury")], PROGRAM_IDS.treasury),
  oracle: (authority: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("oracle"), authority.toBuffer()], PROGRAM_IDS.oracle),
  settlement: (authority: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("settlement"), authority.toBuffer()], PROGRAM_IDS.settlement),
  proposal: (creator: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("proposal"), creator.toBuffer()], PROGRAM_IDS.governance),
};

export const math = {
  ltv: (obligations: number, collateralValue: number) => {
    if (collateralValue === 0) return 0;
    return Math.floor((obligations * 10_000) / collateralValue);
  },
  financingFromCollateral: (collateral: number, mBps: number) => {
    const denom = 10_000 - mBps;
    return Math.floor((collateral * mBps) / denom);
  },
};

export async function fetchOraclePrice(pair: string): Promise<number> {
  const baseUrl = ENDPOINTS.oracle.replace(/\/$/, "");
  const response = await fetch(`${baseUrl}/price?pair=${encodeURIComponent(pair)}`);
  if (!response.ok) {
    throw new Error(`Failed to fetch oracle price for ${pair}: ${response.statusText}`);
  }
  const data = (await response.json()) as { price: number };
  if (typeof data.price !== "number") {
    throw new Error(`Oracle response missing price for ${pair}`);
  }
  return data.price;
}


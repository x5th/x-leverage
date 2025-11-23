import * as anchor from "@coral-xyz/anchor";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const programs = [
    "financing_engine",
    "liquidation_engine",
    "lp_vault",
    "treasury_engine",
    "oracle_framework",
    "governance",
    "settlement_engine",
  ];

  for (const name of programs) {
    console.log(`Deploying ${name}...`);
    await anchor.workspace[name.replace(/(^|_)([a-z])/g, (_, __, c) => c.toUpperCase())];
  }
  console.log("Deployment flow finished (Anchor handles per IDL build/deploy).");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

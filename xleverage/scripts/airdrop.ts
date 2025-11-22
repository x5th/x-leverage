import { Connection, PublicKey, LAMPORTS_PER_SOL, Keypair } from "@solana/web3.js";

async function airdrop() {
  const connection = new Connection(process.env.ANCHOR_PROVIDER_URL || "http://localhost:8899", "confirmed");
  const keypair = Keypair.generate();
  const sig = await connection.requestAirdrop(keypair.publicKey, 2 * LAMPORTS_PER_SOL);
  await connection.confirmTransaction(sig);
  console.log(`Airdropped 2 SOL to ${keypair.publicKey.toBase58()}`);
}

airdrop().catch((err) => {
  console.error(err);
  process.exit(1);
});

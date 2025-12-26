/**
 * Populate UI with real positions and LP deposits
 */

import { Connection, PublicKey, Keypair, Transaction, SystemProgram } from '@solana/web3.js';
import {
    createMint,
    getOrCreateAssociatedTokenAccount,
    mintTo,
    getAssociatedTokenAddress,
    TOKEN_PROGRAM_ID
} from '@solana/spl-token';
import * as fs from 'fs';
import * as path from 'path';

const RPC_URL = 'https://rpc.testnet.x1.xyz';
const LP_VAULT_PROGRAM_ID = new PublicKey('BKCWUpTk3B1yXoFAWugnmLM5s2S1HWpmNiAE3ZJQn5eE');
const USDC_MINT = new PublicKey('2U7gLYjR5o9AMHHvVogNQFiM16gcL93KznRuvScam3S7');
const LP_TOKEN_MINT = new PublicKey('21xrHcaQStV8DhrEATmMgZDDLZoPNqJ5jiB7YMxJN1k6');

async function main() {
    console.log('🚀 Populating UI with Real Data\n');
    console.log('=' .repeat(60));

    const connection = new Connection(RPC_URL, 'confirmed');

    // Load test wallet
    const walletPath = path.join(__dirname, '..', 'test-wallet.json');
    const wallet = Keypair.fromSecretKey(
        new Uint8Array(JSON.parse(fs.readFileSync(walletPath, 'utf8')))
    );

    console.log('\n👛 Wallet:', wallet.publicKey.toBase58());

    // Check SOL balance
    const solBalance = await connection.getBalance(wallet.publicKey);
    console.log('💰 SOL Balance:', (solBalance / 1e9).toFixed(4), 'SOL');

    if (solBalance < 1e9) {
        console.log('\n⚠️  Low SOL balance! You may need more SOL for transaction fees.');
    }

    // Step 1: Check USDC balance
    console.log('\n' + '='.repeat(60));
    console.log('📊 Step 1: Checking USDC Balance');
    console.log('='.repeat(60));

    const userUsdcATA = await getAssociatedTokenAddress(
        USDC_MINT,
        wallet.publicKey,
        false,
        TOKEN_PROGRAM_ID
    );

    console.log('User USDC ATA:', userUsdcATA.toBase58());

    let usdcBalance = 0;
    try {
        const usdcAccount = await connection.getTokenAccountBalance(userUsdcATA);
        usdcBalance = parseInt(usdcAccount.value.amount);
        console.log('✅ Current USDC Balance:', (usdcBalance / 1e6).toLocaleString(), 'USDC');
    } catch (e) {
        console.log('⚠️  USDC account not found. Will create and mint...');
    }

    // Step 2: Mint USDC if needed
    const targetUsdcBalance = 100_000 * 1e6; // 100,000 USDC

    if (usdcBalance < targetUsdcBalance) {
        console.log('\n' + '='.repeat(60));
        console.log('💵 Step 2: Minting USDC');
        console.log('='.repeat(60));

        const amountToMint = targetUsdcBalance - usdcBalance;
        console.log('Minting:', (amountToMint / 1e6).toLocaleString(), 'USDC');

        try {
            // Get or create ATA
            const usdcTokenAccount = await getOrCreateAssociatedTokenAccount(
                connection,
                wallet,
                USDC_MINT,
                wallet.publicKey
            );

            console.log('Token account:', usdcTokenAccount.address.toBase58());

            // Mint USDC
            const signature = await mintTo(
                connection,
                wallet,
                USDC_MINT,
                usdcTokenAccount.address,
                wallet, // mint authority
                amountToMint
            );

            console.log('✅ Minted USDC!');
            console.log('Transaction:', `https://explorer.testnet.x1.xyz/tx/${signature}`);

            usdcBalance = targetUsdcBalance;
            console.log('💰 New USDC Balance:', (usdcBalance / 1e6).toLocaleString(), 'USDC');

        } catch (error: any) {
            console.error('❌ Failed to mint USDC:', error.message);
            throw error;
        }
    } else {
        console.log('\n✅ Sufficient USDC balance:', (usdcBalance / 1e6).toLocaleString(), 'USDC');
    }

    // Step 3: Deposit into LP Vault
    console.log('\n' + '='.repeat(60));
    console.log('🏦 Step 3: Depositing into LP Vault');
    console.log('='.repeat(60));

    const depositAmount = 50_000; // 50,000 USDC
    console.log('Deposit amount:', depositAmount.toLocaleString(), 'USDC');

    // Load LP vault IDL
    const idlPath = path.join(__dirname, '..', 'target', 'idl', 'lp_vault.json');
    const idl = JSON.parse(fs.readFileSync(idlPath, 'utf8'));

    // Derive vault PDA
    const [vaultPDA] = PublicKey.findProgramAddressSync(
        [Buffer.from('vault')],
        LP_VAULT_PROGRAM_ID
    );

    console.log('Vault PDA:', vaultPDA.toBase58());

    // Get ATAs
    const userLpTokenATA = await getAssociatedTokenAddress(
        LP_TOKEN_MINT,
        wallet.publicKey,
        false,
        TOKEN_PROGRAM_ID
    );
    const vaultUsdcATA = await getAssociatedTokenAddress(
        USDC_MINT,
        vaultPDA,
        true, // allowOwnerOffCurve for PDA
        TOKEN_PROGRAM_ID
    );

    console.log('User LP Token ATA:', userLpTokenATA.toBase58());
    console.log('Vault USDC ATA:', vaultUsdcATA.toBase58());

    // Create LP token account if it doesn't exist
    try {
        const lpTokenAccountInfo = await connection.getAccountInfo(userLpTokenATA);
        if (!lpTokenAccountInfo) {
            console.log('\n📝 Creating LP token account...');
            const lpTokenAccount = await getOrCreateAssociatedTokenAccount(
                connection,
                wallet,
                LP_TOKEN_MINT,
                wallet.publicKey
            );
            console.log('✅ LP token account created:', lpTokenAccount.address.toBase58());
        } else {
            console.log('✅ LP token account already exists');
        }
    } catch (error: any) {
        console.error('⚠️  Error checking/creating LP token account:', error.message);
    }

    try {
        // Find deposit instruction
        const depositInstruction = idl.instructions.find((ix: any) => ix.name === 'deposit_usdc');
        if (!depositInstruction) {
            throw new Error('deposit_usdc instruction not found');
        }

        const discriminator = Buffer.from(depositInstruction.discriminator);
        const amountLamports = BigInt(depositAmount * 1e6);

        // Encode amount as u64 (little endian)
        const amountBuffer = Buffer.alloc(8);
        amountBuffer.writeBigUInt64LE(amountLamports);

        const instructionData = Buffer.concat([discriminator, amountBuffer]);

        console.log('\nBuilding deposit transaction...');
        console.log('Amount (lamports):', amountLamports.toString());

        // Build instruction
        const keys = [
            { pubkey: vaultPDA, isSigner: false, isWritable: true },
            { pubkey: LP_TOKEN_MINT, isSigner: false, isWritable: true },
            { pubkey: userLpTokenATA, isSigner: false, isWritable: true },
            { pubkey: userUsdcATA, isSigner: false, isWritable: true },
            { pubkey: vaultUsdcATA, isSigner: false, isWritable: true },
            { pubkey: wallet.publicKey, isSigner: true, isWritable: false },
            { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false }
        ];

        const depositIx = {
            programId: LP_VAULT_PROGRAM_ID,
            keys,
            data: instructionData
        };

        const tx = new Transaction().add(depositIx);
        const { blockhash } = await connection.getLatestBlockhash();
        tx.recentBlockhash = blockhash;
        tx.feePayer = wallet.publicKey;
        tx.sign(wallet);

        console.log('📤 Sending deposit transaction...');

        const signature = await connection.sendRawTransaction(tx.serialize(), {
            skipPreflight: false,
            preflightCommitment: 'confirmed'
        });

        console.log('Transaction sent:', signature);
        await connection.confirmTransaction(signature, 'confirmed');

        console.log('\n✅ Deposit successful!');
        console.log('Transaction:', `https://explorer.testnet.x1.xyz/tx/${signature}`);

        // Check LP token balance
        const lpTokenAccount = await connection.getTokenAccountBalance(userLpTokenATA);
        console.log('💎 LP Token Balance:', lpTokenAccount.value.uiAmountString, 'xLVG-LP');

    } catch (error: any) {
        console.error('❌ Deposit failed:', error.message);
        if (error.logs) {
            console.error('\nTransaction logs:');
            error.logs.forEach((log: string) => console.error('  ', log));
        }
        throw error;
    }

    // Step 4: Summary
    console.log('\n' + '='.repeat(60));
    console.log('📊 Summary');
    console.log('='.repeat(60));

    console.log('\n✅ UI Data Population Complete!');
    console.log('\nYou can now:');
    console.log('  1. Open http://localhost:3000 in your browser');
    console.log('  2. Navigate to LP Vault page');
    console.log('  3. See your deposit in the Overview tab');
    console.log('  4. Check your LP token balance');
    console.log('  5. View transaction history');

    console.log('\n💡 Next steps:');
    console.log('  - Try withdrawing some LP tokens');
    console.log('  - Create positions to see them in the dashboard');
    console.log('  - Monitor vault utilization');

}

main().catch(error => {
    console.error('\n💥 Fatal error:', error);
    process.exit(1);
});

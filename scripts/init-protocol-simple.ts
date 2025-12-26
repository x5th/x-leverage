/**
 * Simple protocol config initialization using deployed program
 */

import { Connection, PublicKey, Keypair, SystemProgram, Transaction, TransactionInstruction } from '@solana/web3.js';
import * as fs from 'fs';
import * as path from 'path';

const RPC_URL = 'https://rpc.testnet.x1.xyz';
const FINANCING_PROGRAM_ID = new PublicKey('2VmBchqNd9gv5g1f9d4bkuJ23yPaEfThGKJ7k3QjbgVr');

async function main() {
    console.log('🚀 Initializing Protocol Config\n');

    // Load wallet
    const walletPath = path.join(__dirname, '..', 'test-wallet.json');
    const wallet = Keypair.fromSecretKey(
        new Uint8Array(JSON.parse(fs.readFileSync(walletPath, 'utf8')))
    );

    console.log('Admin Wallet:', wallet.publicKey.toBase58());

    const connection = new Connection(RPC_URL, 'confirmed');

    // Derive protocol_config PDA
    const [protocolConfigPDA, bump] = PublicKey.findProgramAddressSync(
        [Buffer.from('protocol_config')],
        FINANCING_PROGRAM_ID
    );

    console.log('Protocol Config PDA:', protocolConfigPDA.toBase58());
    console.log('Bump:', bump);

    // Check if already exists
    const accountInfo = await connection.getAccountInfo(protocolConfigPDA);
    if (accountInfo) {
        console.log('\n✅ Protocol config already exists!');
        console.log('Owner:', accountInfo.owner.toBase58());
        console.log('Data length:', accountInfo.data.length, 'bytes');

        // Check if paused (byte at offset 33)
        const paused = accountInfo.data.length >= 34 ? accountInfo.data[33] === 1 : false;
        console.log('Paused:', paused);
        return;
    }

    // Load IDL to get instruction discriminator
    const idlPath = path.join(__dirname, '..', 'target', 'idl', 'financing_engine.json');
    const idl = JSON.parse(fs.readFileSync(idlPath, 'utf8'));

    const initInstruction = idl.instructions.find((ix: any) => ix.name === 'initialize_protocol_config');
    if (!initInstruction) {
        throw new Error('initialize_protocol_config instruction not found in IDL');
    }

    const discriminator = Buffer.from(initInstruction.discriminator);

    console.log('\n⚙️  Creating initialization transaction...');
    console.log('Instruction discriminator:', discriminator.toString('hex'));

    // Build instruction
    const keys = [
        { pubkey: protocolConfigPDA, isSigner: false, isWritable: true },
        { pubkey: wallet.publicKey, isSigner: true, isWritable: true },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false }
    ];

    const instruction = new TransactionInstruction({
        programId: FINANCING_PROGRAM_ID,
        keys,
        data: discriminator
    });

    // Send transaction
    const transaction = new Transaction().add(instruction);
    const { blockhash } = await connection.getLatestBlockhash();
    transaction.recentBlockhash = blockhash;
    transaction.feePayer = wallet.publicKey;
    transaction.sign(wallet);

    try {
        const signature = await connection.sendRawTransaction(transaction.serialize(), {
            skipPreflight: false,
            preflightCommitment: 'confirmed'
        });

        console.log('📤 Transaction sent:', signature);
        await connection.confirmTransaction(signature, 'confirmed');

        console.log('\n✅ Protocol config initialized successfully!');
        console.log('Transaction:', `https://explorer.testnet.x1.xyz/tx/${signature}`);
        console.log('PDA:', protocolConfigPDA.toBase58());
        console.log('Admin:', wallet.publicKey.toBase58());

    } catch (error: any) {
        console.error('\n❌ Initialization failed:', error.message);
        if (error.logs) {
            console.error('\nTransaction logs:');
            error.logs.forEach((log: string) => console.error('  ', log));
        }
        throw error;
    }
}

main().catch(console.error);

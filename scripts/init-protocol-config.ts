import * as anchor from '@coral-xyz/anchor';
import { PublicKey } from '@solana/web3.js';
import * as fs from 'fs';

async function main() {
    console.log('🚀 Initializing Protocol Config...\n');

    // Setup provider
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    console.log('Admin wallet:', provider.wallet.publicKey.toBase58());

    // Load IDL
    const idl = JSON.parse(fs.readFileSync('./target/idl/financing_engine.json', 'utf8'));
    const programId = new PublicKey('2VmBchqNd9gv5g1f9d4bkuJ23yPaEfThGKJ7k3QjbgVr'); // Correct financing engine program ID

    // Create program interface
    const program = new anchor.Program(idl as anchor.Idl, programId, provider);

    // Derive protocol_config PDA
    const [protocolConfig, bump] = PublicKey.findProgramAddressSync(
        [Buffer.from('protocol_config')],
        programId
    );

    console.log('Protocol Config PDA:', protocolConfig.toBase58());
    console.log('Bump:', bump);

    // Check if already initialized
    try {
        const config: any = await provider.connection.getAccountInfo(protocolConfig);
        if (config) {
            console.log('\n✅ Protocol config already initialized!');
            console.log('Owner:', config.owner.toBase58());
            console.log('Data length:', config.data.length, 'bytes');
            return;
        }
    } catch (e) {
        // Account doesn't exist, continue with initialization
    }

    console.log('\n⚙️  Initializing protocol config...');

    try {
        const tx = await program.methods
            .initializeProtocolConfig()
            .accounts({
                protocolConfig: protocolConfig,
                admin: provider.wallet.publicKey,
                systemProgram: anchor.web3.SystemProgram.programId,
            })
            .rpc();

        console.log('\n✅ Protocol config initialized successfully!');
        console.log('Transaction signature:', tx);
        console.log('PDA:', protocolConfig.toBase58());
        console.log('Admin:', provider.wallet.publicKey.toBase58());
        console.log('Paused: false');

    } catch (error: any) {
        console.error('\n❌ Initialization failed:');
        console.error(error);
        if (error.logs) {
            console.error('\nProgram logs:');
            error.logs.forEach((log: string) => console.error(log));
        }
        throw error;
    }
}

main().catch(console.error);

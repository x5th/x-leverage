mod common;

use anchor_lang::prelude::AccountSerialize;
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::token::spl_token;
use common::setup::{mint_data, token_account_data};
use lp_vault::{LPVaultState, VaultError};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::bpf_loader;
use solana_sdk::instruction::{Instruction, InstructionError};
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}

fn lp_vault_processor<'a, 'b, 'c, 'd>(
    program_id: &'a solana_program::pubkey::Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    lp_vault::entry(program_id, accounts, data)
}

#[tokio::test]
async fn test_pause_vault_requires_authority() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    let admin = Keypair::new();
    let attacker = Keypair::new();

    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: 0,
                vault_usdc_balance: 0,
                locked_for_financing: 0,
                utilization: 0,
                authority: admin.pubkey(),
                paused: false,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let context = program_test.start_with_context().await;
    let fund_attacker = system_instruction::transfer(
        &context.payer.pubkey(),
        &attacker.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_attacker],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let accounts = lp_vault::accounts::AdminVaultAction {
        vault: vault_pda,
        authority: attacker.pubkey(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::PauseVault {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&attacker.pubkey()),
        &[&attacker],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized pause should fail");
    let expected = u32::from(VaultError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_allocate_financing_rejected_when_paused() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    let admin = Keypair::new();
    let user = Keypair::new();
    let financed_mint = solana_program::pubkey::Pubkey::new_unique();
    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let vault_token_ata = solana_program::pubkey::Pubkey::new_unique();
    let user_financed_ata = solana_program::pubkey::Pubkey::new_unique();

    program_test.add_account(
        spl_token::id(),
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: bpf_loader::id(),
            executable: true,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: 0,
                vault_usdc_balance: 10_000,
                locked_for_financing: 0,
                utilization: 0,
                authority: admin.pubkey(),
                paused: true,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        financed_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(admin.pubkey()),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_token_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, vault_pda, 10_000),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, user.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::AllocateFinancing {
        vault: vault_pda,
        financed_mint,
        vault_token_ata,
        user_financed_ata,
        token_program: spl_token::id(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::AllocateFinancing { amount: 1_000 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("paused vault should reject financing");
    let expected = u32::from(VaultError::VaultPaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

mod common;

use anchor_lang::prelude::AccountSerialize;
use anchor_lang::prelude::AccountDeserialize;
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
use solana_sdk::system_program;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;
use spl_token::state::Account as TokenAccount;

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

fn add_spl_token_program(program_test: &mut ProgramTest) {
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
}

async fn fetch_vault_state(
    context: &mut solana_program_test::ProgramTestContext,
    vault: solana_program::pubkey::Pubkey,
) -> LPVaultState {
    let account = context
        .banks_client
        .get_account(vault)
        .await
        .expect("get vault account")
        .expect("vault account missing");
    let mut data_slice: &[u8] = &account.data;
    LPVaultState::try_deserialize(&mut data_slice).expect("deserialize vault")
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

#[tokio::test]
async fn test_initialize_vault() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    let authority = Keypair::new();
    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());

    let mut context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::InitializeVault {
        vault: vault_pda,
        payer: context.payer.pubkey(),
        system_program: system_program::id(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::InitializeVault {
            authority: authority.pubkey(),
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert_eq!(vault_state.authority, authority.pubkey());
    assert_eq!(vault_state.total_shares, 0);
    assert_eq!(vault_state.vault_usdc_balance, 0);
    assert_eq!(vault_state.locked_for_financing, 0);
    assert_eq!(vault_state.utilization, 0);
    assert!(!vault_state.paused);
}

#[tokio::test]
async fn test_allocate_financing_liquidity_check() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    add_spl_token_program(&mut program_test);

    let admin = Keypair::new();
    let financed_mint = solana_program::pubkey::Pubkey::new_unique();
    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let vault_token_ata = solana_program::pubkey::Pubkey::new_unique();
    let user_financed_ata = solana_program::pubkey::Pubkey::new_unique();

    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: 0,
                vault_usdc_balance: 1_000,
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
            data: token_account_data(financed_mint, vault_pda, 1_000),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, admin.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
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
        data: lp_vault::instruction::AllocateFinancing { amount: 2_000 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("insufficient liquidity should reject financing");
    let expected = u32::from(VaultError::InsufficientLiquidity);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::AllocateFinancing { amount: 500 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert_eq!(vault_state.vault_usdc_balance, 500);
    assert_eq!(vault_state.locked_for_financing, 500);
}

#[tokio::test]
async fn test_release_financing_accounting() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    add_spl_token_program(&mut program_test);

    let user = Keypair::new();
    let financed_mint = solana_program::pubkey::Pubkey::new_unique();
    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let vault_token_ata = solana_program::pubkey::Pubkey::new_unique();
    let user_financed_ata = solana_program::pubkey::Pubkey::new_unique();

    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: 0,
                vault_usdc_balance: 200,
                locked_for_financing: 300,
                utilization: 0,
                authority: Keypair::new().pubkey(),
                paused: false,
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
            data: mint_data(user.pubkey()),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_token_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, vault_pda, 200),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, user.pubkey(), 300),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::ReleaseFinancing {
        vault: vault_pda,
        financed_mint,
        vault_token_ata,
        user_financed_ata,
        user: user.pubkey(),
        token_program: spl_token::id(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::ReleaseFinancing { amount: 250 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &user],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert_eq!(vault_state.vault_usdc_balance, 450);
    assert_eq!(vault_state.locked_for_financing, 50);
}

#[tokio::test]
async fn test_write_off_bad_debt_authorization() {
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
                vault_usdc_balance: 2_000,
                locked_for_financing: 1_000,
                utilization: 0,
                authority: admin.pubkey(),
                paused: false,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::WriteOffBadDebt {
        vault: vault_pda,
        authority: attacker.pubkey(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::WriteOffBadDebt {
            financing_amount: 800,
            bad_debt: 400,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &attacker],
        context.last_blockhash,
    );
    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized write off should fail");
    let expected = u32::from(VaultError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let accounts = lp_vault::accounts::WriteOffBadDebt {
        vault: vault_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::WriteOffBadDebt {
            financing_amount: 800,
            bad_debt: 400,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &admin],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert_eq!(vault_state.vault_usdc_balance, 1_600);
    assert_eq!(vault_state.locked_for_financing, 200);
}

#[tokio::test]
async fn test_pause_vault_operations() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    let admin = Keypair::new();
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

    let mut context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::AdminVaultAction {
        vault: vault_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::PauseVault {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &admin],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert!(vault_state.paused);

    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::PauseVault {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &admin],
        context.last_blockhash,
    );
    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("re-pausing should fail");
    let expected = u32::from(VaultError::AlreadyPaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::UnpauseVault {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &admin],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert!(!vault_state.paused);

    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::UnpauseVault {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &admin],
        context.last_blockhash,
    );
    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unpausing when not paused should fail");
    let expected = u32::from(VaultError::NotPaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_share_price_calculation() {
    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));

    add_spl_token_program(&mut program_test);

    let user = Keypair::new();
    let usdc_mint = solana_program::pubkey::Pubkey::new_unique();
    let lp_mint = solana_program::pubkey::Pubkey::new_unique();
    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let user_usdc_account = solana_program::pubkey::Pubkey::new_unique();
    let vault_usdc_account = solana_program::pubkey::Pubkey::new_unique();
    let user_lp_account = solana_program::pubkey::Pubkey::new_unique();

    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: 0,
                vault_usdc_balance: 0,
                locked_for_financing: 0,
                utilization: 0,
                authority: Keypair::new().pubkey(),
                paused: false,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        usdc_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(user.pubkey()),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        lp_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(vault_pda),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_usdc_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(usdc_mint, user.pubkey(), 5_000),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_usdc_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(usdc_mint, vault_pda, 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_lp_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(lp_mint, user.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::DepositUsdc {
        vault: vault_pda,
        lp_token_mint: lp_mint,
        user_lp_token_account: user_lp_account,
        user_usdc_account,
        vault_usdc_account,
        user: user.pubkey(),
        token_program: spl_token::id(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::DepositUsdc { amount: 1_000 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &user],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let vault_state = fetch_vault_state(&mut context, vault_pda).await;
    assert_eq!(vault_state.total_shares, 1_000);
    assert_eq!(vault_state.vault_usdc_balance, 1_000);

    let user_lp = context
        .banks_client
        .get_account(user_lp_account)
        .await
        .expect("get user lp account")
        .expect("user lp account missing");
    let user_lp_state = TokenAccount::unpack(&user_lp.data).expect("unpack user lp");
    assert_eq!(user_lp_state.amount, 1_000);

    let mut program_test =
        ProgramTest::new("lp_vault", lp_vault::id(), solana_program_test::processor!(lp_vault_processor));
    add_spl_token_program(&mut program_test);

    let user = Keypair::new();
    let usdc_mint = solana_program::pubkey::Pubkey::new_unique();
    let lp_mint = solana_program::pubkey::Pubkey::new_unique();
    let (vault_pda, _) = solana_program::pubkey::Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let user_usdc_account = solana_program::pubkey::Pubkey::new_unique();
    let vault_usdc_account = solana_program::pubkey::Pubkey::new_unique();
    let user_lp_account = solana_program::pubkey::Pubkey::new_unique();

    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: u64::MAX,
                vault_usdc_balance: 1,
                locked_for_financing: 0,
                utilization: 0,
                authority: Keypair::new().pubkey(),
                paused: false,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        usdc_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(user.pubkey()),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        lp_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(vault_pda),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_usdc_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(usdc_mint, user.pubkey(), u64::MAX),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_usdc_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(usdc_mint, vault_pda, 1),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_lp_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(lp_mint, user.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let accounts = lp_vault::accounts::DepositUsdc {
        vault: vault_pda,
        lp_token_mint: lp_mint,
        user_lp_token_account: user_lp_account,
        user_usdc_account,
        vault_usdc_account,
        user: user.pubkey(),
        token_program: spl_token::id(),
    };
    let ix = Instruction {
        program_id: lp_vault::id(),
        accounts: accounts.to_account_metas(None),
        data: lp_vault::instruction::DepositUsdc {
            amount: u64::MAX,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &user],
        context.last_blockhash,
    );
    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("overflow should reject share calculation");
    let expected = u32::from(VaultError::MathOverflow);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

mod common;

use anchor_lang::prelude::{AccountDeserialize, AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use oracle_framework::{OracleError, OracleSource, OracleState};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::instruction::{Instruction, InstructionError};
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::system_program;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}

fn oracle_framework_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    oracle_framework::entry(program_id, accounts, data)
}

fn add_oracle_account(program_test: &mut ProgramTest, oracle: Pubkey, state: OracleState) {
    program_test.add_account(
        oracle,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&state),
            owner: oracle_framework::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
}

async fn fund_signer(context: &mut solana_program_test::ProgramTestContext, signer: &Pubkey) {
    let fund_ix = system_instruction::transfer(&context.payer.pubkey(), signer, 1_000_000_000);
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();
}

#[tokio::test]
async fn test_initialize_oracle_global_pda() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let protocol_admin = Pubkey::new_unique();
    let (oracle_pda, _) = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id());

    let mut context = program_test.start_with_context().await;
    let accounts = oracle_framework::accounts::InitializeOracle {
        oracle: oracle_pda,
        authority: context.payer.pubkey(),
        system_program: system_program::id(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::InitializeOracle { protocol_admin }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let account = context
        .banks_client
        .get_account(oracle_pda)
        .await
        .expect("get oracle account")
        .expect("oracle account not found");
    let mut data: &[u8] = &account.data;
    let state = OracleState::try_deserialize(&mut data).expect("deserialize oracle state");

    assert_eq!(state.authority, context.payer.pubkey());
    assert_eq!(state.protocol_admin, protocol_admin);
    assert_eq!(state.pyth_price, 0);
    assert!(!state.paused);
}

#[tokio::test]
async fn test_update_price_authorization() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let admin = Keypair::new();
    let attacker = Keypair::new();
    let oracle_pda = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id()).0;

    add_oracle_account(
        &mut program_test,
        oracle_pda,
        OracleState {
            authority: admin.pubkey(),
            protocol_admin: admin.pubkey(),
            pyth_price: 1,
            switchboard_price: 1,
            synthetic_twap: 1,
            last_twap_window: 0,
            frozen_price: 0,
            frozen_slot: 0,
            last_update_slot: 0,
            paused: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &attacker.pubkey()).await;

    let accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: attacker.pubkey(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::UpdateOraclePrice {
            source: OracleSource::Pyth,
            price: 5,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&attacker.pubkey()),
        &[&attacker],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized update should fail");
    let expected = u32::from(OracleError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_price_bounds_validation() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let admin = Keypair::new();
    let oracle_pda = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id()).0;
    add_oracle_account(
        &mut program_test,
        oracle_pda,
        OracleState {
            authority: admin.pubkey(),
            protocol_admin: admin.pubkey(),
            pyth_price: 1,
            switchboard_price: 1,
            synthetic_twap: 1,
            last_twap_window: 0,
            frozen_price: 0,
            frozen_slot: 0,
            last_update_slot: 0,
            paused: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin.pubkey()).await;

    let max_price = i64::MAX / 10_000;
    let accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::UpdateOraclePrice {
            source: OracleSource::Pyth,
            price: max_price,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("out of bounds price should fail");
    let expected = u32::from(OracleError::PriceOutOfBounds);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_staleness_detection() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let admin = Keypair::new();
    let oracle_pda = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id()).0;
    add_oracle_account(
        &mut program_test,
        oracle_pda,
        OracleState {
            authority: admin.pubkey(),
            protocol_admin: admin.pubkey(),
            pyth_price: 1,
            switchboard_price: 1,
            synthetic_twap: 1,
            last_twap_window: 0,
            frozen_price: 0,
            frozen_slot: 0,
            last_update_slot: 0,
            paused: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin.pubkey()).await;
    context.warp_to_slot(200).unwrap();

    let accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::FreezeSnapshotForLiquidation {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("stale snapshot should fail");
    let expected = u32::from(OracleError::StalePrice);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_calculate_twap_authorization() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let admin = Keypair::new();
    let attacker = Keypair::new();
    let oracle_pda = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id()).0;
    add_oracle_account(
        &mut program_test,
        oracle_pda,
        OracleState {
            authority: admin.pubkey(),
            protocol_admin: admin.pubkey(),
            pyth_price: 1,
            switchboard_price: 1,
            synthetic_twap: 1,
            last_twap_window: 0,
            frozen_price: 0,
            frozen_slot: 0,
            last_update_slot: 0,
            paused: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &attacker.pubkey()).await;

    let accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: attacker.pubkey(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::CalculateTwap { window: 50 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&attacker.pubkey()),
        &[&attacker],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized TWAP should fail");
    let expected = u32::from(OracleError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_freeze_snapshot_authorization() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let admin = Keypair::new();
    let attacker = Keypair::new();
    let oracle_pda = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id()).0;
    add_oracle_account(
        &mut program_test,
        oracle_pda,
        OracleState {
            authority: admin.pubkey(),
            protocol_admin: admin.pubkey(),
            pyth_price: 1,
            switchboard_price: 1,
            synthetic_twap: 1,
            last_twap_window: 0,
            frozen_price: 0,
            frozen_slot: 0,
            last_update_slot: 0,
            paused: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &attacker.pubkey()).await;

    let accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: attacker.pubkey(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::FreezeSnapshotForLiquidation {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&attacker.pubkey()),
        &[&attacker],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized freeze should fail");
    let expected = u32::from(OracleError::UnauthorizedFreeze);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_pause_oracle_updates() {
    let mut program_test = ProgramTest::new(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );

    let admin = Keypair::new();
    let oracle_pda = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id()).0;
    add_oracle_account(
        &mut program_test,
        oracle_pda,
        OracleState {
            authority: admin.pubkey(),
            protocol_admin: admin.pubkey(),
            pyth_price: 1,
            switchboard_price: 1,
            synthetic_twap: 1,
            last_twap_window: 0,
            frozen_price: 0,
            frozen_slot: 0,
            last_update_slot: 0,
            paused: true,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin.pubkey()).await;

    let accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: accounts.to_account_metas(None),
        data: oracle_framework::instruction::UpdateOraclePrice {
            source: OracleSource::Pyth,
            price: 5,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("paused oracle should reject updates");
    let expected = u32::from(OracleError::OraclePaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

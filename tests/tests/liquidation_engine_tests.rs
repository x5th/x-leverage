use anchor_lang::prelude::{AccountDeserialize, AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use liquidation_engine::{LiquidationAuthority, LiquidationError};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::instruction::{Instruction, InstructionError};
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_program;
use solana_sdk::transaction::{Transaction, TransactionError};

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}

fn liquidation_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    liquidation_engine::entry(program_id, accounts, data)
}

fn add_liquidation_authority(
    program_test: &mut ProgramTest,
    owner: Pubkey,
    delegated_liquidator: Pubkey,
    frozen_snapshot_slot: u64,
    frozen_price: u64,
    executed: bool,
) -> Pubkey {
    let (authority_pda, _) = Pubkey::find_program_address(
        &[b"liquidation", owner.as_ref()],
        &liquidation_engine::id(),
    );
    let authority = LiquidationAuthority {
        owner,
        delegated_liquidator,
        frozen_snapshot_slot,
        frozen_price,
        executed,
        last_fee_accrued: 0,
        last_user_return: 0,
    };
    program_test.add_account(
        authority_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&authority),
            owner: liquidation_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    authority_pda
}

#[tokio::test]
async fn test_snapshot_expiration() {
    let mut program_test = ProgramTest::new(
        "liquidation_engine",
        liquidation_engine::id(),
        solana_program_test::processor!(liquidation_engine_processor),
    );

    let owner = Keypair::new();
    let delegated_liquidator = Pubkey::new_unique();
    let oracle_feed = Pubkey::new_unique();
    let authority_pda = add_liquidation_authority(
        &mut program_test,
        owner.pubkey(),
        delegated_liquidator,
        0,
        0,
        false,
    );
    program_test.add_account(
        oracle_feed,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;

    let accounts = liquidation_engine::accounts::FreezeOracleSnapshot {
        authority: authority_pda,
        oracle_feed,
    };
    let ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: liquidation_engine::instruction::FreezeOracleSnapshot { price: 155 }.data(),
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
        .get_account(authority_pda)
        .await
        .expect("get authority account")
        .expect("authority account");
    let mut data_slice: &[u8] = &account.data;
    let authority = LiquidationAuthority::try_deserialize(&mut data_slice).expect("deserialize authority");
    let frozen_slot = authority.frozen_snapshot_slot;

    let ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: liquidation_engine::instruction::FreezeOracleSnapshot { price: 200 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.banks_client.get_latest_blockhash().await.unwrap(),
    );
    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("double snapshot should fail before expiry");
    let expected = u32::from(LiquidationError::DoubleLiquidation);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    context
        .warp_to_slot(frozen_slot + 101)
        .await
        .expect("warp to future slot");

    let ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: liquidation_engine::instruction::FreezeOracleSnapshot { price: 250 }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.banks_client.get_latest_blockhash().await.unwrap(),
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let account = context
        .banks_client
        .get_account(authority_pda)
        .await
        .expect("get authority account")
        .expect("authority account");
    let mut data_slice: &[u8] = &account.data;
    let refreshed = LiquidationAuthority::try_deserialize(&mut data_slice).expect("deserialize authority");
    assert!(refreshed.frozen_snapshot_slot > frozen_slot);
    assert_eq!(refreshed.frozen_price, 250);
}

#[tokio::test]
async fn test_delegated_liquidator_validation() {
    let mut program_test = ProgramTest::new(
        "liquidation_engine",
        liquidation_engine::id(),
        solana_program_test::processor!(liquidation_engine_processor),
    );

    let owner = Keypair::new();
    let delegated_liquidator = Keypair::new();
    let unauthorized = Keypair::new();
    let dex_router = Pubkey::new_unique();

    let authority_pda = add_liquidation_authority(
        &mut program_test,
        owner.pubkey(),
        delegated_liquidator.pubkey(),
        1,
        1_000,
        false,
    );
    program_test.add_account(
        unauthorized.pubkey(),
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        dex_router,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let context = program_test.start_with_context().await;
    let accounts = liquidation_engine::accounts::ExecuteLiquidation {
        authority: authority_pda,
        delegated_liquidator: unauthorized.pubkey(),
        dex_router,
    };
    let ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: liquidation_engine::instruction::ExecuteLiquidation {
            ltv: 10_000,
            liquidation_threshold: 9_000,
            slippage_bps: 0,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &unauthorized],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized liquidator should fail");
    let expected = u32::from(LiquidationError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let account = context
        .banks_client
        .get_account(authority_pda)
        .await
        .expect("get authority account")
        .expect("authority account");
    let mut data_slice: &[u8] = &account.data;
    let authority = LiquidationAuthority::try_deserialize(&mut data_slice).expect("deserialize authority");
    assert!(!authority.executed);
}

#[tokio::test]
async fn test_state_reset_after_execution() {
    let mut program_test = ProgramTest::new(
        "liquidation_engine",
        liquidation_engine::id(),
        solana_program_test::processor!(liquidation_engine_processor),
    );

    let owner = Keypair::new();
    let delegated_liquidator = Pubkey::new_unique();
    let authority_pda = add_liquidation_authority(
        &mut program_test,
        owner.pubkey(),
        delegated_liquidator,
        10,
        10_000,
        true,
    );

    let context = program_test.start_with_context().await;
    let accounts = liquidation_engine::accounts::DistributeLiquidationProceeds {
        authority: authority_pda,
    };
    let ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: liquidation_engine::instruction::DistributeLiquidationProceeds {
            total_proceeds: 10_000,
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

    let account = context
        .banks_client
        .get_account(authority_pda)
        .await
        .expect("get authority account")
        .expect("authority account");
    let mut data_slice: &[u8] = &account.data;
    let authority = LiquidationAuthority::try_deserialize(&mut data_slice).expect("deserialize authority");
    assert_eq!(authority.last_fee_accrued, 300);
    assert_eq!(authority.last_user_return, 9_700);
    assert_eq!(authority.frozen_snapshot_slot, 0);
    assert_eq!(authority.frozen_price, 0);
    assert!(!authority.executed);
}

#[tokio::test]
async fn test_slippage_limits() {
    let mut program_test = ProgramTest::new(
        "liquidation_engine",
        liquidation_engine::id(),
        solana_program_test::processor!(liquidation_engine_processor),
    );

    let owner = Keypair::new();
    let delegated_liquidator = Keypair::new();
    let oracle_feed = Pubkey::new_unique();
    let dex_router = Pubkey::new_unique();
    let authority_pda = add_liquidation_authority(
        &mut program_test,
        owner.pubkey(),
        delegated_liquidator.pubkey(),
        0,
        0,
        false,
    );
    for account in [oracle_feed, dex_router] {
        program_test.add_account(
            account,
            Account {
                lamports: 1_000_000,
                data: vec![],
                owner: system_program::id(),
                executable: false,
                rent_epoch: 0,
            },
        );
    }
    program_test.add_account(
        delegated_liquidator.pubkey(),
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let freeze_accounts = liquidation_engine::accounts::FreezeOracleSnapshot {
        authority: authority_pda,
        oracle_feed,
    };
    let freeze_ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: freeze_accounts.to_account_metas(None),
        data: liquidation_engine::instruction::FreezeOracleSnapshot { price: 150 }.data(),
    };
    let freeze_tx = Transaction::new_signed_with_payer(
        &[freeze_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(freeze_tx).await.unwrap();

    let execute_accounts = liquidation_engine::accounts::ExecuteLiquidation {
        authority: authority_pda,
        delegated_liquidator: delegated_liquidator.pubkey(),
        dex_router,
    };
    let execute_ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: execute_accounts.to_account_metas(None),
        data: liquidation_engine::instruction::ExecuteLiquidation {
            ltv: 10_000,
            liquidation_threshold: 9_000,
            slippage_bps: 250,
        }
        .data(),
    };
    let execute_tx = Transaction::new_signed_with_payer(
        &[execute_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &delegated_liquidator],
        context.banks_client.get_latest_blockhash().await.unwrap(),
    );

    let result = context.banks_client.process_transaction(execute_tx).await;
    let err = result.expect_err("slippage limit should be enforced");
    let expected = u32::from(LiquidationError::SlippageTooHigh);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let account = context
        .banks_client
        .get_account(authority_pda)
        .await
        .expect("get authority account")
        .expect("authority account");
    let mut data_slice: &[u8] = &account.data;
    let authority = LiquidationAuthority::try_deserialize(&mut data_slice).expect("deserialize authority");
    assert!(!authority.executed);
}

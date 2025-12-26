use anchor_lang::prelude::{AccountDeserialize, AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClient, BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::instruction::{Instruction, InstructionError};
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;
use treasury_engine::{Treasury, TreasuryError};

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}

fn treasury_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    treasury_engine::entry(program_id, accounts, data)
}

async fn fetch_treasury(banks_client: &mut BanksClient, treasury: Pubkey) -> Treasury {
    let account = banks_client
        .get_account(treasury)
        .await
        .expect("get treasury")
        .expect("treasury missing");
    let mut data: &[u8] = &account.data;
    Treasury::try_deserialize(&mut data).expect("deserialize treasury")
}

async fn fund_signer(context: &mut solana_program_test::ProgramTestContext, signer: &Keypair) {
    let fund_ix = system_instruction::transfer(
        &context.payer.pubkey(),
        &signer.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(fund_tx)
        .await
        .expect("fund signer");
}

#[tokio::test]
async fn test_allocate_requires_admin_and_updates() {
    let mut program_test = ProgramTest::new(
        "treasury_engine",
        treasury_engine::id(),
        solana_program_test::processor!(treasury_engine_processor),
    );

    let admin = Keypair::new();
    let attacker = Keypair::new();
    let (treasury_pda, _) = Pubkey::find_program_address(&[b"treasury"], &treasury_engine::id());

    program_test.add_account(
        treasury_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&Treasury {
                admin: admin.pubkey(),
                lp_contributed: 1_000_000,
                co_financing_outstanding: 0,
                base_fee_accrued: 0,
                carry_accrued: 0,
                compounded_xrs: 0,
                paused: false,
            }),
            owner: treasury_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin).await;
    fund_signer(&mut context, &attacker).await;

    let accounts = treasury_engine::accounts::TreasuryCtx {
        treasury: treasury_pda,
        authority: attacker.pubkey(),
    };
    let ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: treasury_engine::instruction::TreasuryAllocate {
            co_finance_amount: 100_000,
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
    let err = result.expect_err("unauthorized allocate should fail");
    let expected = u32::from(TreasuryError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let treasury = fetch_treasury(&mut context.banks_client, treasury_pda).await;
    assert_eq!(treasury.co_financing_outstanding, 0);

    let accounts = treasury_engine::accounts::TreasuryCtx {
        treasury: treasury_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: treasury_engine::instruction::TreasuryAllocate {
            co_finance_amount: 100_000,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(tx)
        .await
        .expect("authorized allocate should succeed");

    let treasury = fetch_treasury(&mut context.banks_client, treasury_pda).await;
    assert_eq!(treasury.co_financing_outstanding, 100_000);
}

#[tokio::test]
async fn test_co_financing_limits_enforced() {
    let mut program_test = ProgramTest::new(
        "treasury_engine",
        treasury_engine::id(),
        solana_program_test::processor!(treasury_engine_processor),
    );

    let admin = Keypair::new();
    let (treasury_pda, _) = Pubkey::find_program_address(&[b"treasury"], &treasury_engine::id());

    program_test.add_account(
        treasury_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&Treasury {
                admin: admin.pubkey(),
                lp_contributed: 1_000,
                co_financing_outstanding: 400,
                base_fee_accrued: 0,
                carry_accrued: 0,
                compounded_xrs: 0,
                paused: false,
            }),
            owner: treasury_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin).await;

    let accounts = treasury_engine::accounts::TreasuryCtx {
        treasury: treasury_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: treasury_engine::instruction::TreasuryAllocate {
            co_finance_amount: 200,
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
    let err = result.expect_err("co-finance limit should be enforced");
    let expected = u32::from(TreasuryError::CoFinanceLimit);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let treasury = fetch_treasury(&mut context.banks_client, treasury_pda).await;
    assert_eq!(treasury.co_financing_outstanding, 400);
}

#[tokio::test]
async fn test_compound_resets_yield_balances() {
    let mut program_test = ProgramTest::new(
        "treasury_engine",
        treasury_engine::id(),
        solana_program_test::processor!(treasury_engine_processor),
    );

    let admin = Keypair::new();
    let (treasury_pda, _) = Pubkey::find_program_address(&[b"treasury"], &treasury_engine::id());

    program_test.add_account(
        treasury_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&Treasury {
                admin: admin.pubkey(),
                lp_contributed: 0,
                co_financing_outstanding: 0,
                base_fee_accrued: 100,
                carry_accrued: 50,
                compounded_xrs: 1_000,
                paused: false,
            }),
            owner: treasury_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin).await;

    let accounts = treasury_engine::accounts::TreasuryCtx {
        treasury: treasury_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: treasury_engine::instruction::TreasuryCompoundXrs {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(tx)
        .await
        .expect("compound should succeed");

    let treasury = fetch_treasury(&mut context.banks_client, treasury_pda).await;
    assert_eq!(treasury.compounded_xrs, 1_045);
    assert_eq!(treasury.base_fee_accrued, 0);
    assert_eq!(treasury.carry_accrued, 0);
}

#[tokio::test]
async fn test_pause_blocks_allocate() {
    let mut program_test = ProgramTest::new(
        "treasury_engine",
        treasury_engine::id(),
        solana_program_test::processor!(treasury_engine_processor),
    );

    let admin = Keypair::new();
    let (treasury_pda, _) = Pubkey::find_program_address(&[b"treasury"], &treasury_engine::id());

    program_test.add_account(
        treasury_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&Treasury {
                admin: admin.pubkey(),
                lp_contributed: 1_000,
                co_financing_outstanding: 0,
                base_fee_accrued: 0,
                carry_accrued: 0,
                compounded_xrs: 0,
                paused: false,
            }),
            owner: treasury_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &admin).await;

    let accounts = treasury_engine::accounts::AdminTreasuryAction {
        treasury: treasury_pda,
        admin_authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: treasury_engine::instruction::PauseTreasury {}.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(tx)
        .await
        .expect("pause should succeed");

    let treasury = fetch_treasury(&mut context.banks_client, treasury_pda).await;
    assert!(treasury.paused);

    let accounts = treasury_engine::accounts::TreasuryCtx {
        treasury: treasury_pda,
        authority: admin.pubkey(),
    };
    let ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: treasury_engine::instruction::TreasuryAllocate {
            co_finance_amount: 100,
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
    let err = result.expect_err("paused treasury should reject allocate");
    let expected = u32::from(TreasuryError::TreasuryPaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let treasury = fetch_treasury(&mut context.banks_client, treasury_pda).await;
    assert!(treasury.paused);
    assert_eq!(treasury.co_financing_outstanding, 0);
}

mod common;

use anchor_lang::prelude::{AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::token::spl_token;
use common::setup::{mint_data, token_account_data};
use financing_engine::{FinancingState, PositionStatus, ProtocolConfig, UserPositionCounter};
use lp_vault::LPVaultState;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::instruction::Instruction;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::TransactionError;
use solana_sdk::transaction::Transaction;

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}


fn financing_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    financing_engine::entry(program_id, accounts, data)
}

fn lp_vault_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    lp_vault::entry(program_id, accounts, data)
}

#[tokio::test]
async fn test_close_at_maturity_rejects_invalid_vault_financed_owner() {
    let mut program_test = ProgramTest::new(
        "financing_engine",
        financing_engine::id(),
        solana_program_test::processor!(financing_engine_processor),
    );
    program_test.add_program(
        "lp_vault",
        lp_vault::id(),
        solana_program_test::processor!(lp_vault_processor),
    );
    program_test.add_program(
        "spl_token",
        spl_token::id(),
        solana_program_test::processor!(spl_token::processor::Processor::process),
    );

    let user = Keypair::new();
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[b"financing", user.pubkey().as_ref(), collateral_mint.as_ref()],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", user.pubkey().as_ref()],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) = Pubkey::find_program_address(
        &[b"protocol_config"],
        &financing_engine::id(),
    );
    let (vault_authority_pda, _) = Pubkey::find_program_address(
        &[b"vault_authority"],
        &financing_engine::id(),
    );
    let (vault_collateral_ata, _) = Pubkey::find_program_address(
        &[b"vault_collateral", collateral_mint.as_ref()],
        &financing_engine::id(),
    );
    let (user_collateral_ata, _) = Pubkey::find_program_address(
        &[b"user_collateral", user.pubkey().as_ref(), collateral_mint.as_ref()],
        &financing_engine::id(),
    );
    let (vault_financed_ata, _) = Pubkey::find_program_address(
        &[b"vault_financed", financed_mint.as_ref()],
        &financing_engine::id(),
    );
    let (user_financed_ata, _) = Pubkey::find_program_address(
        &[b"user_financed", user.pubkey().as_ref(), financed_mint.as_ref()],
        &financing_engine::id(),
    );
    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"lp_vault"], &lp_vault::id());

    program_test.add_account(
        protocol_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&ProtocolConfig {
                admin_authority: admin.pubkey(),
                protocol_paused: false,
            }),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        state_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&FinancingState {
                user_pubkey: user.pubkey(),
                collateral_mint,
                collateral_amount: 5_000,
                collateral_usd_value: 100_000_000,
                financing_amount: 10_000,
                initial_ltv: 5_000,
                max_ltv: 8_000,
                term_start: 0,
                term_end: 0,
                fee_schedule: 0,
                carry_enabled: false,
                liquidation_threshold: 0,
                oracle_sources: Vec::new(),
                delegated_settlement_authority: Pubkey::default(),
                delegated_liquidation_authority: Pubkey::default(),
                position_status: PositionStatus::Active,
            }),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        position_counter_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&UserPositionCounter {
                user: user.pubkey(),
                open_positions: 1,
            }),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        lp_vault_state,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                authority: admin.pubkey(),
                paused: false,
                vault_usdc_balance: 0,
                locked_for_financing: 10_000,
                total_shares: 0,
                utilization: 0,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        collateral_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(admin.pubkey()),
            owner: spl_token::id(),
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
        vault_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, vault_authority_pda, 5_000),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, user.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, vault_authority_pda, 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, user.pubkey(), 10_000),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_authority_pda,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let context = program_test.start_with_context().await;
    let fund_user = system_instruction::transfer(
        &context.payer.pubkey(),
        &user.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_user],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let accounts = financing_engine::accounts::CloseAtMaturity {
        state: state_pda,
        collateral_mint,
        vault_collateral_ata,
        user_collateral_ata,
        vault_authority: vault_authority_pda,
        receiver: user.pubkey(),
        position_counter: position_counter_pda,
        token_program: spl_token::id(),
        lp_vault: lp_vault_state,
        financed_mint,
        vault_financed_ata,
        user_financed_ata,
        lp_vault_program: lp_vault::id(),
        protocol_config: protocol_config_pda,
    };
    let ix = Instruction {
        program_id: financing_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: financing_engine::instruction::CloseAtMaturity {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&user.pubkey()),
        &[&user],
        context.last_blockhash,
    );
    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("invalid vault owner should fail");
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, 2003, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

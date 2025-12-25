mod common;

use anchor_lang::prelude::{AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::token::spl_token;
use common::setup::{mint_data, token_account_data};
use financing_engine::{FinancingError, FinancingState, PositionStatus, ProtocolConfig, UserPositionCounter};
use lp_vault::LPVaultState;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::bpf_loader;
use solana_sdk::instruction::Instruction;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;

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

struct CloseAtMaturityFixture {
    state_pda: Pubkey,
    position_counter_pda: Pubkey,
    protocol_config_pda: Pubkey,
    vault_authority_pda: Pubkey,
    collateral_mint: Pubkey,
    financed_mint: Pubkey,
    vault_collateral_ata: Pubkey,
    user_collateral_ata: Pubkey,
    lp_vault_state: Pubkey,
    vault_financed_ata: Pubkey,
    user_financed_ata: Pubkey,
}

fn add_close_at_maturity_accounts(
    program_test: &mut ProgramTest,
    owner: &Keypair,
    receiver: Pubkey,
    protocol_paused: bool,
    user_financed_amount: u64,
    financing_amount: u64,
) -> CloseAtMaturityFixture {
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[b"financing", owner.pubkey().as_ref(), collateral_mint.as_ref()],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", owner.pubkey().as_ref()],
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

    let lp_vault_state = Pubkey::new_unique();
    let vault_collateral_ata = Pubkey::new_unique();
    let user_collateral_ata = Pubkey::new_unique();
    let vault_financed_ata = Pubkey::new_unique();
    let user_financed_ata = Pubkey::new_unique();

    let protocol_config = ProtocolConfig {
        admin_authority: admin.pubkey(),
        protocol_paused,
    };
    program_test.add_account(
        protocol_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&protocol_config),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let financing_state = FinancingState {
        user_pubkey: owner.pubkey(),
        collateral_mint,
        collateral_amount: 0,
        collateral_usd_value: 100_000_000,
        financing_amount,
        initial_ltv: 5_000,
        max_ltv: 8_000,
        term_start: 0,
        term_end: -1,
        fee_schedule: 0,
        carry_enabled: false,
        liquidation_threshold: 9_000,
        oracle_sources: vec![],
        delegated_settlement_authority: Pubkey::default(),
        delegated_liquidation_authority: Pubkey::default(),
        position_status: PositionStatus::Active,
    };
    program_test.add_account(
        state_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&financing_state),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let position_counter = UserPositionCounter {
        user: owner.pubkey(),
        open_positions: 1,
    };
    program_test.add_account(
        position_counter_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&position_counter),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let lp_vault_state_data = LPVaultState {
        total_shares: 0,
        vault_usdc_balance: 0,
        locked_for_financing: 0,
        utilization: 0,
        authority: admin.pubkey(),
        paused: false,
    };
    program_test.add_account(
        lp_vault_state,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&lp_vault_state_data),
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
            data: token_account_data(collateral_mint, vault_authority_pda, 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, receiver, 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        vault_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, lp_vault_state, 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, receiver, user_financed_amount),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    program_test.add_account(
        lp_vault::id(),
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: bpf_loader::id(),
            executable: true,
            rent_epoch: 0,
        },
    );
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
        vault_authority_pda,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    CloseAtMaturityFixture {
        state_pda,
        position_counter_pda,
        protocol_config_pda,
        vault_authority_pda,
        collateral_mint,
        financed_mint,
        vault_collateral_ata,
        user_collateral_ata,
        lp_vault_state,
        vault_financed_ata,
        user_financed_ata,
    }
}

async fn submit_close_at_maturity(
    program_test: ProgramTest,
    signer: &Keypair,
    receiver: Pubkey,
    fixture: CloseAtMaturityFixture,
) -> Result<(), BanksClientError> {
    let mut context = program_test.start_with_context().await;

    let fund_signer = system_instruction::transfer(
        &context.payer.pubkey(),
        &signer.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_signer],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let accounts = financing_engine::accounts::CloseAtMaturity {
        state: fixture.state_pda,
        collateral_mint: fixture.collateral_mint,
        vault_collateral_ata: fixture.vault_collateral_ata,
        user_collateral_ata: fixture.user_collateral_ata,
        vault_authority: fixture.vault_authority_pda,
        receiver,
        position_counter: fixture.position_counter_pda,
        token_program: spl_token::id(),
        lp_vault: fixture.lp_vault_state,
        financed_mint: fixture.financed_mint,
        vault_financed_ata: fixture.vault_financed_ata,
        user_financed_ata: fixture.user_financed_ata,
        lp_vault_program: lp_vault::id(),
        protocol_config: fixture.protocol_config_pda,
    };

    let ix = Instruction {
        program_id: financing_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: financing_engine::instruction::CloseAtMaturity {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&signer.pubkey()),
        &[signer],
        context.last_blockhash,
    );

    context.banks_client.process_transaction(tx).await
}

#[tokio::test]
async fn test_vuln_007_unauthorized_close_position() {
    let mut program_test = ProgramTest::new(
        "financing_engine",
        financing_engine::id(),
        solana_program_test::processor!(financing_engine_processor),
    );

    let alice = Keypair::new();
    let bob = Keypair::new();

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        bob.pubkey(),
        false,
        0,
        0,
    );

    let result = submit_close_at_maturity(program_test, &bob, bob.pubkey(), fixture).await;
    let err = result.expect_err("unauthorized close should fail");
    let expected = u32::from(FinancingError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_close_at_maturity_rejects_insufficient_repayment() {
    let mut program_test = ProgramTest::new(
        "financing_engine",
        financing_engine::id(),
        solana_program_test::processor!(financing_engine_processor),
    );

    let alice = Keypair::new();

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        false,
        0,
        10_000,
    );

    let result = submit_close_at_maturity(program_test, &alice, alice.pubkey(), fixture).await;
    let err = result.expect_err("repayment should fail");
    let expected = u32::from(FinancingError::InsufficientBalanceForClosure);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_close_at_maturity_rejected_when_paused() {
    let mut program_test = ProgramTest::new(
        "financing_engine",
        financing_engine::id(),
        solana_program_test::processor!(financing_engine_processor),
    );

    let alice = Keypair::new();

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        true,
        0,
        0,
    );

    let result = submit_close_at_maturity(program_test, &alice, alice.pubkey(), fixture).await;
    let err = result.expect_err("paused protocol should fail");
    let expected = u32::from(FinancingError::ProtocolPaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

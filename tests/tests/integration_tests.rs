mod common;

use anchor_lang::prelude::{AccountDeserialize, AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::associated_token::ID as ASSOCIATED_TOKEN_PROGRAM_ID;
use anchor_spl::token::spl_token;
use common::setup::{mint_data, oracle_sources, token_account_data, MIN_COLLATERAL_USD, MIN_FINANCING_AMOUNT};
use financing_engine::{FinancingState, PositionStatus, ProtocolConfig, UserPositionCounter};
use governance::{GovernanceConfig, Proposal, VoteRecord};
use liquidation_engine::LiquidationAuthority;
use lp_vault::LPVaultState;
use oracle_framework::OracleState;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::bpf_loader;
use solana_sdk::instruction::Instruction;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::TransactionError;
use solana_sdk::transaction::Transaction;
use treasury_engine::Treasury;

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}

fn deserialize_anchor_account<T: AccountDeserialize>(account: &Account) -> T {
    T::try_deserialize(&mut account.data.as_slice()).expect("deserialize account")
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

fn oracle_framework_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    oracle_framework::entry(program_id, accounts, data)
}

fn governance_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    governance::entry(program_id, accounts, data)
}

fn liquidation_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    liquidation_engine::entry(program_id, accounts, data)
}

fn treasury_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    treasury_engine::entry(program_id, accounts, data)
}

fn integration_program_test() -> ProgramTest {
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
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );
    program_test.add_program(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );
    program_test.add_program(
        "liquidation_engine",
        liquidation_engine::id(),
        solana_program_test::processor!(liquidation_engine_processor),
    );
    program_test.add_program(
        "treasury_engine",
        treasury_engine::id(),
        solana_program_test::processor!(treasury_engine_processor),
    );
    program_test.add_program(
        "spl_token",
        spl_token::id(),
        solana_program_test::processor!(spl_token::processor::Processor::process),
    );
    program_test.add_account(
        ASSOCIATED_TOKEN_PROGRAM_ID,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: bpf_loader::id(),
            executable: true,
            rent_epoch: 0,
        },
    );
    program_test
}

fn associated_token_address(owner: Pubkey, mint: Pubkey) -> Pubkey {
    let (address, _) = Pubkey::find_program_address(
        &[owner.as_ref(), spl_token::id().as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    address
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

#[tokio::test]
async fn test_full_position_lifecycle() {
    let mut program_test = integration_program_test();
    let user = Keypair::new();
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();
    let oracle_accounts = Pubkey::new_unique();

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
    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());

    let vault_collateral_ata = associated_token_address(vault_authority_pda, collateral_mint);
    let user_financed_ata = associated_token_address(user.pubkey(), financed_mint);
    let user_collateral_ata = Pubkey::new_unique();
    let vault_financed_ata = Pubkey::new_unique();

    let collateral_amount = 5_000;
    let financing_amount = MIN_FINANCING_AMOUNT;

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
        lp_vault_state,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                authority: admin.pubkey(),
                paused: false,
                vault_usdc_balance: 200_000_000,
                locked_for_financing: 0,
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
        user_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, user.pubkey(), collateral_amount),
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
        vault_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, lp_vault_state, 200_000_000),
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
    program_test.add_account(
        oracle_accounts,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: oracle_framework::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let fund_user = system_instruction::transfer(
        &context.payer.pubkey(),
        &user.pubkey(),
        1_000_000_000,
    );
    let fund_admin = system_instruction::transfer(
        &context.payer.pubkey(),
        &admin.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_user, fund_admin],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let open_accounts = financing_engine::accounts::InitializeFinancing {
        state: state_pda,
        collateral_mint,
        user_collateral_ata,
        vault_collateral_ata,
        vault_authority: vault_authority_pda,
        oracle_accounts,
        user: user.pubkey(),
        position_counter: position_counter_pda,
        token_program: spl_token::id(),
        associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
        system_program: solana_sdk::system_program::id(),
        lp_vault: lp_vault_state,
        financed_mint,
        vault_financed_ata,
        user_financed_ata,
        lp_vault_program: lp_vault::id(),
        protocol_config: protocol_config_pda,
    };
    let open_ix = Instruction {
        program_id: financing_engine::id(),
        accounts: open_accounts.to_account_metas(None),
        data: financing_engine::instruction::InitializeFinancing {
            collateral_amount,
            collateral_usd_value: MIN_COLLATERAL_USD,
            financing_amount,
            initial_ltv: 5_000,
            max_ltv: 8_000,
            term_start: -100,
            term_end: -50,
            fee_schedule: 0,
            carry_enabled: false,
            liquidation_threshold: 8_500,
            oracle_sources: oracle_sources(),
        }
        .data(),
    };
    let open_tx = Transaction::new_signed_with_payer(
        &[open_ix],
        Some(&user.pubkey()),
        &[&user],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(open_tx).await.unwrap();

    let update_accounts = financing_engine::accounts::UpdateLtv {
        state: state_pda,
        protocol_config: protocol_config_pda,
        authority: admin.pubkey(),
    };
    let update_ix = Instruction {
        program_id: financing_engine::id(),
        accounts: update_accounts.to_account_metas(None),
        data: financing_engine::instruction::UpdateLtv {
            collateral_usd_value: 150_000_000,
        }
        .data(),
    };
    let update_tx = Transaction::new_signed_with_payer(
        &[update_ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(update_tx).await.unwrap();

    let close_accounts = financing_engine::accounts::CloseAtMaturity {
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
    let close_ix = Instruction {
        program_id: financing_engine::id(),
        accounts: close_accounts.to_account_metas(None),
        data: financing_engine::instruction::CloseAtMaturity {}.data(),
    };
    let close_tx = Transaction::new_signed_with_payer(
        &[close_ix],
        Some(&user.pubkey()),
        &[&user],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(close_tx).await.unwrap();

    let state_account = context
        .banks_client
        .get_account(state_pda)
        .await
        .unwrap()
        .expect("state account");
    let state = deserialize_anchor_account::<FinancingState>(&state_account);
    assert_eq!(state.position_status, PositionStatus::Closed);

    let counter_account = context
        .banks_client
        .get_account(position_counter_pda)
        .await
        .unwrap()
        .expect("counter account");
    let counter = deserialize_anchor_account::<UserPositionCounter>(&counter_account);
    assert_eq!(counter.open_positions, 0);

    let vault_account = context
        .banks_client
        .get_account(lp_vault_state)
        .await
        .unwrap()
        .expect("vault account");
    let vault = deserialize_anchor_account::<LPVaultState>(&vault_account);
    assert_eq!(vault.locked_for_financing, 0);
    assert_eq!(vault.vault_usdc_balance, 200_000_000);
}

#[tokio::test]
async fn test_liquidation_flow() {
    let mut program_test = integration_program_test();
    let user = Keypair::new();
    let admin = Keypair::new();
    let oracle_authority = Keypair::new();
    let liquidator = Keypair::new();
    let oracle_feed = Pubkey::new_unique();

    let (oracle_pda, _) = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id());
    let (liquidation_authority_pda, _) = Pubkey::find_program_address(
        &[b"liquidation", user.pubkey().as_ref()],
        &liquidation_engine::id(),
    );
    let (protocol_config_pda, _) = Pubkey::find_program_address(
        &[b"protocol_config"],
        &financing_engine::id(),
    );
    let (state_pda, _) = Pubkey::find_program_address(
        &[b"financing", user.pubkey().as_ref(), oracle_feed.as_ref()],
        &financing_engine::id(),
    );

    let financing_state = FinancingState {
        user_pubkey: user.pubkey(),
        collateral_mint: oracle_feed,
        collateral_amount: 0,
        collateral_usd_value: 200_000_000,
        financing_amount: 150_000_000,
        initial_ltv: 5_000,
        max_ltv: 8_000,
        term_start: 0,
        term_end: 0,
        fee_schedule: 0,
        carry_enabled: false,
        liquidation_threshold: 8_000,
        oracle_sources: vec![oracle_authority.pubkey()],
        delegated_settlement_authority: Pubkey::default(),
        delegated_liquidation_authority: Pubkey::default(),
        position_status: PositionStatus::Active,
    };

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
            data: serialize_anchor_account(&financing_state),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        oracle_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&OracleState {
                authority: oracle_authority.pubkey(),
                protocol_admin: admin.pubkey(),
                pyth_price: 0,
                switchboard_price: 0,
                synthetic_twap: 0,
                last_twap_window: 0,
                frozen_price: 0,
                frozen_slot: 0,
                last_update_slot: 0,
                paused: false,
            }),
            owner: oracle_framework::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        liquidation_authority_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LiquidationAuthority {
                owner: user.pubkey(),
                delegated_liquidator: liquidator.pubkey(),
                frozen_snapshot_slot: 0,
                frozen_price: 0,
                executed: false,
                last_fee_accrued: 0,
                last_user_return: 0,
            }),
            owner: liquidation_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        oracle_feed,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: oracle_framework::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let fund_tx = Transaction::new_signed_with_payer(
        &[
            system_instruction::transfer(&context.payer.pubkey(), &oracle_authority.pubkey(), 1_000_000_000),
            system_instruction::transfer(&context.payer.pubkey(), &liquidator.pubkey(), 1_000_000_000),
        ],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let update_oracle_accounts = oracle_framework::accounts::OracleCtx {
        oracle: oracle_pda,
        authority: oracle_authority.pubkey(),
    };
    let update_oracle_ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: update_oracle_accounts.to_account_metas(None),
        data: oracle_framework::instruction::UpdateOraclePrice {
            source: oracle_framework::OracleSource::Pyth,
            price: 100_000_000,
        }
        .data(),
    };
    let update_oracle_tx = Transaction::new_signed_with_payer(
        &[update_oracle_ix],
        Some(&oracle_authority.pubkey()),
        &[&oracle_authority],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(update_oracle_tx).await.unwrap();

    let oracle_account = context
        .banks_client
        .get_account(oracle_pda)
        .await
        .unwrap()
        .expect("oracle account");
    let oracle_state = deserialize_anchor_account::<OracleState>(&oracle_account);
    assert_eq!(oracle_state.pyth_price, 100_000_000);

    let ltv = financing_engine::ltv_model(
        financing_state.financing_amount,
        oracle_state.pyth_price as u64,
    )
    .expect("ltv");

    let freeze_accounts = liquidation_engine::accounts::FreezeOracleSnapshot {
        authority: liquidation_authority_pda,
        oracle_feed,
    };
    let freeze_ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: freeze_accounts.to_account_metas(None),
        data: liquidation_engine::instruction::FreezeOracleSnapshot { price: oracle_state.pyth_price as u64 }.data(),
    };
    let freeze_tx = Transaction::new_signed_with_payer(
        &[freeze_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(freeze_tx).await.unwrap();

    let execute_accounts = liquidation_engine::accounts::ExecuteLiquidation {
        authority: liquidation_authority_pda,
        delegated_liquidator: liquidator.pubkey(),
        dex_router: Pubkey::new_unique(),
    };
    let execute_ix = Instruction {
        program_id: liquidation_engine::id(),
        accounts: execute_accounts.to_account_metas(None),
        data: liquidation_engine::instruction::ExecuteLiquidation {
            ltv,
            liquidation_threshold: financing_state.liquidation_threshold,
            slippage_bps: 100,
        }
        .data(),
    };
    let execute_tx = Transaction::new_signed_with_payer(
        &[execute_ix],
        Some(&liquidator.pubkey()),
        &[&liquidator],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(execute_tx).await.unwrap();

    let liquidation_account = context
        .banks_client
        .get_account(liquidation_authority_pda)
        .await
        .unwrap()
        .expect("liquidation account");
    let liquidation_state = deserialize_anchor_account::<LiquidationAuthority>(&liquidation_account);
    assert!(liquidation_state.executed);
    assert!(liquidation_state.frozen_snapshot_slot > 0);
}

#[tokio::test]
async fn test_lp_vault_flow() {
    let mut program_test = integration_program_test();
    let user = Keypair::new();
    let (vault_pda, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let usdc_mint = Pubkey::new_unique();
    let lp_token_mint = Pubkey::new_unique();
    let user_usdc_account = Pubkey::new_unique();
    let vault_usdc_account = Pubkey::new_unique();
    let user_lp_token_account = Pubkey::new_unique();
    let user_financed_account = Pubkey::new_unique();

    let deposit_amount = 100_000_000;
    let allocation_amount = 40_000_000;

    program_test.add_account(
        vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                authority: user.pubkey(),
                paused: false,
                vault_usdc_balance: 0,
                locked_for_financing: 0,
                total_shares: 0,
                utilization: 0,
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
        lp_token_mint,
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
            data: token_account_data(usdc_mint, user.pubkey(), deposit_amount),
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
        user_lp_token_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(lp_token_mint, user.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_financed_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(usdc_mint, user.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
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

    let deposit_accounts = lp_vault::accounts::DepositUsdc {
        vault: vault_pda,
        lp_token_mint,
        user_lp_token_account,
        user_usdc_account,
        vault_usdc_account,
        user: user.pubkey(),
        token_program: spl_token::id(),
    };
    let deposit_ix = Instruction {
        program_id: lp_vault::id(),
        accounts: deposit_accounts.to_account_metas(None),
        data: lp_vault::instruction::DepositUsdc {
            amount: deposit_amount,
        }
        .data(),
    };
    let deposit_tx = Transaction::new_signed_with_payer(
        &[deposit_ix],
        Some(&user.pubkey()),
        &[&user],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(deposit_tx).await.unwrap();

    let allocate_accounts = lp_vault::accounts::AllocateFinancing {
        vault: vault_pda,
        financed_mint: usdc_mint,
        vault_token_ata: vault_usdc_account,
        user_financed_ata: user_financed_account,
        token_program: spl_token::id(),
    };
    let allocate_ix = Instruction {
        program_id: lp_vault::id(),
        accounts: allocate_accounts.to_account_metas(None),
        data: lp_vault::instruction::AllocateFinancing {
            amount: allocation_amount,
        }
        .data(),
    };
    let allocate_tx = Transaction::new_signed_with_payer(
        &[allocate_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(allocate_tx).await.unwrap();

    let vault_after_allocate = context
        .banks_client
        .get_account(vault_pda)
        .await
        .unwrap()
        .expect("vault account");
    let vault_state = deserialize_anchor_account::<LPVaultState>(&vault_after_allocate);
    assert_eq!(vault_state.locked_for_financing, allocation_amount);
    assert_eq!(vault_state.vault_usdc_balance, deposit_amount - allocation_amount);

    let release_accounts = lp_vault::accounts::ReleaseFinancing {
        vault: vault_pda,
        financed_mint: usdc_mint,
        vault_token_ata: vault_usdc_account,
        user_financed_ata: user_financed_account,
        user: user.pubkey(),
        token_program: spl_token::id(),
    };
    let release_ix = Instruction {
        program_id: lp_vault::id(),
        accounts: release_accounts.to_account_metas(None),
        data: lp_vault::instruction::ReleaseFinancing {
            amount: allocation_amount,
        }
        .data(),
    };
    let release_tx = Transaction::new_signed_with_payer(
        &[release_ix],
        Some(&user.pubkey()),
        &[&user],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(release_tx).await.unwrap();

    let vault_after_release = context
        .banks_client
        .get_account(vault_pda)
        .await
        .unwrap()
        .expect("vault account");
    let final_vault = deserialize_anchor_account::<LPVaultState>(&vault_after_release);
    assert_eq!(final_vault.locked_for_financing, 0);
    assert_eq!(final_vault.vault_usdc_balance, deposit_amount);
}

#[tokio::test]
async fn test_governance_flow() {
    let mut program_test = integration_program_test();
    let creator = Keypair::new();
    let voter = Keypair::new();
    let xgt_mint = Pubkey::new_unique();
    let user_xgt_account = Pubkey::new_unique();

    let (governance_config_pda, _) =
        Pubkey::find_program_address(&[b"governance_config"], &governance::id());
    let proposal_nonce = 1u64;
    let (proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &proposal_nonce.to_le_bytes()],
        &governance::id(),
    );
    let (vote_record_pda, _) = Pubkey::find_program_address(
        &[b"vote", proposal_pda.as_ref(), voter.pubkey().as_ref()],
        &governance::id(),
    );

    program_test.add_account(
        xgt_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(creator.pubkey()),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        user_xgt_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(xgt_mint, voter.pubkey(), 1_500),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let fund_tx = Transaction::new_signed_with_payer(
        &[
            system_instruction::transfer(&context.payer.pubkey(), &creator.pubkey(), 1_000_000_000),
            system_instruction::transfer(&context.payer.pubkey(), &voter.pubkey(), 1_000_000_000),
        ],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let init_accounts = governance::accounts::InitializeGovernance {
        governance_config: governance_config_pda,
        payer: creator.pubkey(),
        system_program: solana_sdk::system_program::id(),
    };
    let init_ix = Instruction {
        program_id: governance::id(),
        accounts: init_accounts.to_account_metas(None),
        data: governance::instruction::InitializeGovernance {
            quorum_votes: 1_000,
            voting_period: 86_400,
            timelock_delay: 172_800,
            admin_authority: creator.pubkey(),
        }
        .data(),
    };
    let init_tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&creator.pubkey()),
        &[&creator],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(init_tx).await.unwrap();

    let create_accounts = governance::accounts::CreateProposal {
        proposal: proposal_pda,
        governance_config: governance_config_pda,
        creator: creator.pubkey(),
        system_program: solana_sdk::system_program::id(),
    };
    let create_ix = Instruction {
        program_id: governance::id(),
        accounts: create_accounts.to_account_metas(None),
        data: governance::instruction::CreateProposal {
            proposal_nonce,
            title: "Raise LTV cap".to_string(),
            description: "Adjust risk limits".to_string(),
            eta: 0,
        }
        .data(),
    };
    let create_tx = Transaction::new_signed_with_payer(
        &[create_ix],
        Some(&creator.pubkey()),
        &[&creator],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(create_tx).await.unwrap();

    let vote_accounts = governance::accounts::Vote {
        proposal: proposal_pda,
        vote_record: vote_record_pda,
        voter: voter.pubkey(),
        user_xgt_account,
        xgt_mint,
        system_program: solana_sdk::system_program::id(),
        governance_config: governance_config_pda,
    };
    let vote_ix = Instruction {
        program_id: governance::id(),
        accounts: vote_accounts.to_account_metas(None),
        data: governance::instruction::Vote { support: true }.data(),
    };
    let vote_tx = Transaction::new_signed_with_payer(
        &[vote_ix],
        Some(&voter.pubkey()),
        &[&voter],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(vote_tx).await.unwrap();

    let queue_accounts = governance::accounts::QueueExecution {
        proposal: proposal_pda,
        governance_config: governance_config_pda,
    };
    let queue_ix = Instruction {
        program_id: governance::id(),
        accounts: queue_accounts.to_account_metas(None),
        data: governance::instruction::QueueExecution {}.data(),
    };
    let queue_tx = Transaction::new_signed_with_payer(
        &[queue_ix],
        Some(&creator.pubkey()),
        &[&creator],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(queue_tx).await.unwrap();

    let execute_accounts = governance::accounts::ExecuteProposal {
        proposal: proposal_pda,
        governance_config: governance_config_pda,
        executor: creator.pubkey(),
    };
    let execute_ix = Instruction {
        program_id: governance::id(),
        accounts: execute_accounts.to_account_metas(None),
        data: governance::instruction::Execute {}.data(),
    };
    let execute_tx = Transaction::new_signed_with_payer(
        &[execute_ix],
        Some(&creator.pubkey()),
        &[&creator],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(execute_tx).await.unwrap();

    let proposal_account = context
        .banks_client
        .get_account(proposal_pda)
        .await
        .unwrap()
        .expect("proposal account");
    let proposal = deserialize_anchor_account::<Proposal>(&proposal_account);
    assert!(proposal.executed);
    assert!(proposal.for_votes >= 1_000);

    let config_account = context
        .banks_client
        .get_account(governance_config_pda)
        .await
        .unwrap()
        .expect("config account");
    let config = deserialize_anchor_account::<GovernanceConfig>(&config_account);
    assert_eq!(config.proposal_count, 1);

    let vote_account = context
        .banks_client
        .get_account(vote_record_pda)
        .await
        .unwrap()
        .expect("vote record account");
    let vote_record = deserialize_anchor_account::<VoteRecord>(&vote_account);
    assert_eq!(vote_record.weight, 1_500);
}

#[tokio::test]
async fn test_cross_program_circuit_breaker() {
    let mut program_test = integration_program_test();
    let admin = Keypair::new();

    let (protocol_config_pda, _) = Pubkey::find_program_address(
        &[b"protocol_config"],
        &financing_engine::id(),
    );
    let (lp_vault_pda, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let (governance_config_pda, _) =
        Pubkey::find_program_address(&[b"governance_config"], &governance::id());
    let (oracle_pda, _) = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id());
    let (treasury_pda, _) = Pubkey::find_program_address(&[b"treasury"], &treasury_engine::id());

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
        lp_vault_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                authority: admin.pubkey(),
                paused: false,
                vault_usdc_balance: 0,
                locked_for_financing: 0,
                total_shares: 0,
                utilization: 0,
            }),
            owner: lp_vault::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        governance_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&GovernanceConfig {
                quorum_votes: 1_000,
                voting_period: 86_400,
                timelock_delay: 172_800,
                proposal_count: 0,
                admin_authority: admin.pubkey(),
                paused: false,
            }),
            owner: governance::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        oracle_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&OracleState {
                authority: admin.pubkey(),
                protocol_admin: admin.pubkey(),
                pyth_price: 0,
                switchboard_price: 0,
                synthetic_twap: 0,
                last_twap_window: 0,
                frozen_price: 0,
                frozen_slot: 0,
                last_update_slot: 0,
                paused: false,
            }),
            owner: oracle_framework::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        treasury_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&Treasury {
                admin: admin.pubkey(),
                lp_contributed: 0,
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
    let fund_admin = system_instruction::transfer(
        &context.payer.pubkey(),
        &admin.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_admin],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let pause_protocol_ix = Instruction {
        program_id: financing_engine::id(),
        accounts: financing_engine::accounts::AdminProtocolAction {
            protocol_config: protocol_config_pda,
            admin_authority: admin.pubkey(),
        }
        .to_account_metas(None),
        data: financing_engine::instruction::PauseProtocol {}.data(),
    };
    let pause_vault_ix = Instruction {
        program_id: lp_vault::id(),
        accounts: lp_vault::accounts::AdminVaultAction {
            vault: lp_vault_pda,
            authority: admin.pubkey(),
        }
        .to_account_metas(None),
        data: lp_vault::instruction::PauseVault {}.data(),
    };
    let pause_governance_ix = Instruction {
        program_id: governance::id(),
        accounts: governance::accounts::AdminGovernanceAction {
            governance_config: governance_config_pda,
            admin_authority: admin.pubkey(),
        }
        .to_account_metas(None),
        data: governance::instruction::PauseGovernance {}.data(),
    };
    let pause_oracle_ix = Instruction {
        program_id: oracle_framework::id(),
        accounts: oracle_framework::accounts::AdminOracleAction {
            oracle: oracle_pda,
            protocol_admin: admin.pubkey(),
        }
        .to_account_metas(None),
        data: oracle_framework::instruction::PauseOracle {}.data(),
    };
    let pause_treasury_ix = Instruction {
        program_id: treasury_engine::id(),
        accounts: treasury_engine::accounts::AdminTreasuryAction {
            treasury: treasury_pda,
            admin_authority: admin.pubkey(),
        }
        .to_account_metas(None),
        data: treasury_engine::instruction::PauseTreasury {}.data(),
    };

    let pause_tx = Transaction::new_signed_with_payer(
        &[
            pause_protocol_ix,
            pause_vault_ix,
            pause_governance_ix,
            pause_oracle_ix,
            pause_treasury_ix,
        ],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(pause_tx).await.unwrap();

    let protocol_account = context
        .banks_client
        .get_account(protocol_config_pda)
        .await
        .unwrap()
        .expect("protocol config");
    let protocol_config = deserialize_anchor_account::<ProtocolConfig>(&protocol_account);
    assert!(protocol_config.protocol_paused);

    let vault_account = context
        .banks_client
        .get_account(lp_vault_pda)
        .await
        .unwrap()
        .expect("lp vault");
    let vault_state = deserialize_anchor_account::<LPVaultState>(&vault_account);
    assert!(vault_state.paused);

    let governance_account = context
        .banks_client
        .get_account(governance_config_pda)
        .await
        .unwrap()
        .expect("governance config");
    let governance_config = deserialize_anchor_account::<GovernanceConfig>(&governance_account);
    assert!(governance_config.paused);

    let oracle_account = context
        .banks_client
        .get_account(oracle_pda)
        .await
        .unwrap()
        .expect("oracle");
    let oracle_state = deserialize_anchor_account::<OracleState>(&oracle_account);
    assert!(oracle_state.paused);

    let treasury_account = context
        .banks_client
        .get_account(treasury_pda)
        .await
        .unwrap()
        .expect("treasury");
    let treasury_state = deserialize_anchor_account::<Treasury>(&treasury_account);
    assert!(treasury_state.paused);
}

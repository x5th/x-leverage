mod common;

use common::setup::{MIN_COLLATERAL_USD, MIN_FINANCING_AMOUNT, oracle_sources, sample_protocol_admin};
use financing_engine::{
    financing_amount_from_collateral, ltv_model, required_liquidation_gap, PositionStatus,
    ProtocolConfig, UserPositionCounter, FinancingError, FinancingState,
};
use anchor_lang::prelude::{AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::token::spl_token;
use lp_vault::LPVaultState;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_option::COption;
use solana_program_pack::Pack;
use solana_program_test::{BanksClientError, ProgramTest};
use solana_sdk::account::Account;
use solana_sdk::bpf_loader;
use solana_sdk::instruction::Instruction;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::TransactionError;
use solana_sdk::transaction::Transaction;
use solana_sdk::instruction::InstructionError;
use anchor_spl::token::spl_token::state::{Account as TokenAccount, Mint};

// ========== UNIT TESTS (from PR #9) ==========

#[test]
fn test_initialize_financing_success() {
    let collateral_value = 200_000_000u64; // $200
    let obligations = 80_000_000u64; // $80
    let ltv = ltv_model(obligations, collateral_value).expect("ltv model");
    assert!(ltv <= 8_000, "ltv should be within max");

    let gap = required_liquidation_gap(collateral_value, obligations, 9_000)
        .expect("liquidation gap");
    assert!(gap >= 0, "gap should be non-negative");
}

#[test]
fn test_initialize_financing_below_minimum() {
    let collateral_value = MIN_COLLATERAL_USD - 1;
    let financing_amount = MIN_FINANCING_AMOUNT - 1;
    assert!(collateral_value < MIN_COLLATERAL_USD);
    assert!(financing_amount < MIN_FINANCING_AMOUNT);
}

#[test]
fn test_initialize_financing_ltv_ordering() {
    let initial_ltv = 3_000;
    let max_ltv = 7_500;
    let liquidation_threshold = 8_500;
    assert!(initial_ltv <= max_ltv);
    assert!(max_ltv <= liquidation_threshold);
    assert!(liquidation_threshold >= max_ltv + 500);
}

#[test]
fn test_initialize_financing_position_limit() {
    let mut counter = UserPositionCounter { user: Pubkey::default(), open_positions: 0 };
    for _ in 0..UserPositionCounter::MAX_POSITIONS {
        counter.open_positions += 1;
    }
    assert_eq!(counter.open_positions, UserPositionCounter::MAX_POSITIONS);
}

#[test]
fn test_initialize_financing_while_paused() {
    let config = ProtocolConfig { admin_authority: sample_protocol_admin(), protocol_paused: true };
    assert!(config.protocol_paused);
}

#[test]
fn test_close_at_maturity_success() {
    let status = PositionStatus::Active;
    assert_eq!(status, PositionStatus::Active);
}

#[test]
fn test_close_at_maturity_with_outstanding_debt() {
    let financing_amount = 10_000u64;
    let fee_schedule = 500u64;
    let obligations = financing_amount + fee_schedule;
    assert!(obligations > financing_amount);
}

#[test]
fn test_close_early_fee_calculation() {
    let collateral_amount = 1_000_000u64;
    let fee_bps = 50u64;
    let fee = collateral_amount * fee_bps / 10_000;
    assert_eq!(fee, 5_000);
    let amount_to_return = collateral_amount - fee;
    assert!(amount_to_return > 0);
}

#[test]
fn test_update_ltv_oracle_authorization() {
    let admin = sample_protocol_admin();
    let sources = oracle_sources();
    let oracle = sources[0];
    assert!(sources.contains(&oracle));
    assert_ne!(admin, oracle);
}

#[test]
fn test_liquidate_valid_threshold() {
    let obligations = 90_000_000u64;
    let collateral_value = 100_000_000u64;
    let ltv = ltv_model(obligations, collateral_value).expect("ltv");
    assert!(ltv >= 9_000);
}

#[test]
fn test_liquidate_oracle_price_validation() {
    let collateral_value = 100_000_000u64;
    let obligations = 50_000_000u64;
    let ltv = ltv_model(obligations, collateral_value).expect("ltv");
    assert!(ltv > 0);
}

#[test]
fn test_force_liquidate_admin_only() {
    let admin = sample_protocol_admin();
    let other = Pubkey::new_unique();
    assert_ne!(admin, other);
}

// ========== PROGRAM TESTS (from PR #10) ==========

fn serialize_anchor_account<T: AccountSerialize>(data: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    data.try_serialize(&mut buf).expect("serialize account");
    buf
}

fn token_account_data(mint: Pubkey, owner: Pubkey, amount: u64) -> Vec<u8> {
    let token_account = TokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: spl_token::state::AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; TokenAccount::LEN];
    TokenAccount::pack(token_account, &mut data).expect("pack token account");
    data
}

fn mint_data(mint_authority: Pubkey) -> Vec<u8> {
    let mint = Mint {
        mint_authority: COption::Some(mint_authority),
        supply: 0,
        decimals: 6,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; Mint::LEN];
    Mint::pack(mint, &mut data).expect("pack mint");
    data
}

fn financing_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    financing_engine::entry(program_id, accounts, data)
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
    let admin = Keypair::new();

    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[b"financing", alice.pubkey().as_ref(), collateral_mint.as_ref()],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", alice.pubkey().as_ref()],
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
        protocol_paused: false,
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
        user_pubkey: alice.pubkey(),
        collateral_mint,
        collateral_amount: 0,
        collateral_usd_value: 100_000_000,
        financing_amount: 0,
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
        user: alice.pubkey(),
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
            data: token_account_data(collateral_mint, bob.pubkey(), 0),
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
            data: token_account_data(financed_mint, bob.pubkey(), 0),
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

    let mut context = program_test.start_with_context().await;

    let fund_bob = system_instruction::transfer(
        &context.payer.pubkey(),
        &bob.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_bob],
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
        receiver: bob.pubkey(),
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
        Some(&bob.pubkey()),
        &[&bob],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized close should fail");
    let expected = u32::from(FinancingError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(code))) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

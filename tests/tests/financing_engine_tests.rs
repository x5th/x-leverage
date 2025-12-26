mod common;

use anchor_lang::prelude::{AccountDeserialize, AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::token::spl_token;
use common::setup::{mint_data, token_account_data};
use financing_engine::{
    FinancingError, FinancingState, PositionStatus, ProtocolConfig, UserPositionCounter,
};
use lp_vault::LPVaultState;
use oracle_framework::OracleState;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_pack::Pack;
use solana_program_test::{BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::account::Account;
use solana_sdk::instruction::Instruction;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;
use spl_associated_token_account::processor::process_instruction as associated_token_process_instruction;

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

fn setup_program_test() -> ProgramTest {
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
    program_test.add_program(
        "spl_associated_token_account",
        spl_associated_token_account::id(),
        solana_program_test::processor!(associated_token_process_instruction),
    );
    program_test
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

async fn fund_signer(context: &mut ProgramTestContext, signer: &Keypair) {
    let fund_signer =
        system_instruction::transfer(&context.payer.pubkey(), &signer.pubkey(), 1_000_000_000);
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_signer],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(fund_tx)
        .await
        .unwrap();
    context.last_blockhash = context.banks_client.get_latest_blockhash().await.unwrap();
}

fn assert_financing_error(err: BanksClientError, expected: FinancingError) {
    let expected = u32::from(expected);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected, "unexpected error code");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn add_close_at_maturity_accounts(
    program_test: &mut ProgramTest,
    owner: &Keypair,
    receiver: Pubkey,
    protocol_paused: bool,
    user_financed_amount: u64,
    financing_amount: u64,
    collateral_amount: u64,
    fee_schedule: u64,
    term_end: i64,
) -> CloseAtMaturityFixture {
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[
            b"financing",
            owner.pubkey().as_ref(),
            collateral_mint.as_ref(),
        ],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", owner.pubkey().as_ref()],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) =
        Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id());
    let (vault_authority_pda, _) =
        Pubkey::find_program_address(&[b"vault_authority"], &financing_engine::id());

    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
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
        collateral_amount,
        collateral_usd_value: 100_000_000,
        financing_amount,
        initial_ltv: 5_000,
        max_ltv: 8_000,
        term_start: 0,
        term_end,
        fee_schedule,
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
        locked_for_financing: financing_amount,
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
            data: token_account_data(collateral_mint, vault_authority_pda, collateral_amount),
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
    fixture: &CloseAtMaturityFixture,
) -> Result<ProgramTestContext, BanksClientError> {
    let mut context = program_test.start_with_context().await;

    fund_signer(&mut context, signer).await;

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

    context.banks_client.process_transaction(tx).await?;
    Ok(context)
}

struct InitializeFinancingFixture {
    state_pda: Pubkey,
    position_counter_pda: Pubkey,
    protocol_config_pda: Pubkey,
    vault_authority_pda: Pubkey,
    collateral_mint: Pubkey,
    financed_mint: Pubkey,
    user_collateral_ata: Pubkey,
    vault_collateral_ata: Pubkey,
    user_financed_ata: Pubkey,
    vault_financed_ata: Pubkey,
    lp_vault_state: Pubkey,
    oracle_accounts: Pubkey,
}

fn add_initialize_financing_accounts(
    program_test: &mut ProgramTest,
    user: &Keypair,
    collateral_amount: u64,
    financing_amount: u64,
    protocol_paused: bool,
    position_counter: Option<u8>,
) -> InitializeFinancingFixture {
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();
    let oracle_accounts = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[
            b"financing",
            user.pubkey().as_ref(),
            collateral_mint.as_ref(),
        ],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", user.pubkey().as_ref()],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) =
        Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id());
    let (vault_authority_pda, _) =
        Pubkey::find_program_address(&[b"vault_authority"], &financing_engine::id());
    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());

    let user_collateral_ata = Pubkey::new_unique();
    let vault_collateral_ata = Pubkey::new_unique();
    let user_financed_ata = Pubkey::new_unique();
    let vault_financed_ata = Pubkey::new_unique();

    program_test.add_account(
        protocol_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&ProtocolConfig {
                admin_authority: admin.pubkey(),
                protocol_paused,
            }),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    if let Some(open_positions) = position_counter {
        program_test.add_account(
            position_counter_pda,
            Account {
                lamports: 1_000_000,
                data: serialize_anchor_account(&UserPositionCounter {
                    user: user.pubkey(),
                    open_positions,
                }),
                owner: financing_engine::id(),
                executable: false,
                rent_epoch: 0,
            },
        );
    }

    program_test.add_account(
        lp_vault_state,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&LPVaultState {
                total_shares: 0,
                vault_usdc_balance: financing_amount,
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
        vault_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, lp_vault_state, financing_amount),
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

    InitializeFinancingFixture {
        state_pda,
        position_counter_pda,
        protocol_config_pda,
        vault_authority_pda,
        collateral_mint,
        financed_mint,
        user_collateral_ata,
        vault_collateral_ata,
        user_financed_ata,
        vault_financed_ata,
        lp_vault_state,
        oracle_accounts,
    }
}

async fn submit_initialize_financing(
    program_test: ProgramTest,
    signer: &Keypair,
    fixture: &InitializeFinancingFixture,
    collateral_amount: u64,
    collateral_usd_value: u64,
    financing_amount: u64,
    initial_ltv: u64,
    max_ltv: u64,
    liquidation_threshold: u64,
    term_start: i64,
    term_end: i64,
) -> Result<ProgramTestContext, BanksClientError> {
    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, signer).await;

    let accounts = financing_engine::accounts::InitializeFinancing {
        state: fixture.state_pda,
        collateral_mint: fixture.collateral_mint,
        user_collateral_ata: fixture.user_collateral_ata,
        vault_collateral_ata: fixture.vault_collateral_ata,
        vault_authority: fixture.vault_authority_pda,
        oracle_accounts: fixture.oracle_accounts,
        user: signer.pubkey(),
        position_counter: fixture.position_counter_pda,
        token_program: spl_token::id(),
        associated_token_program: spl_associated_token_account::id(),
        system_program: solana_sdk::system_program::id(),
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
        data: financing_engine::instruction::InitializeFinancing {
            collateral_amount,
            collateral_usd_value,
            financing_amount,
            initial_ltv,
            max_ltv,
            term_start,
            term_end,
            fee_schedule: 0,
            carry_enabled: false,
            liquidation_threshold,
            oracle_sources: common::setup::oracle_sources(),
        }
        .data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&signer.pubkey()),
        &[signer],
        context.last_blockhash,
    );

    context.banks_client.process_transaction(tx).await?;
    Ok(context)
}

struct CloseEarlyFixture {
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

fn add_close_early_accounts(
    program_test: &mut ProgramTest,
    owner: &Keypair,
    receiver: Pubkey,
    protocol_paused: bool,
    user_financed_amount: u64,
    financing_amount: u64,
    collateral_amount: u64,
    term_end: i64,
) -> CloseEarlyFixture {
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[
            b"financing",
            owner.pubkey().as_ref(),
            collateral_mint.as_ref(),
        ],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", owner.pubkey().as_ref()],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) =
        Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id());
    let (vault_authority_pda, _) =
        Pubkey::find_program_address(&[b"vault_authority"], &financing_engine::id());
    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());

    let vault_collateral_ata = Pubkey::new_unique();
    let user_collateral_ata = Pubkey::new_unique();
    let vault_financed_ata = Pubkey::new_unique();
    let user_financed_ata = Pubkey::new_unique();

    program_test.add_account(
        protocol_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&ProtocolConfig {
                admin_authority: admin.pubkey(),
                protocol_paused,
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
                user_pubkey: owner.pubkey(),
                collateral_mint,
                collateral_amount,
                collateral_usd_value: 100_000_000,
                financing_amount,
                initial_ltv: 5_000,
                max_ltv: 8_000,
                term_start: 0,
                term_end,
                fee_schedule: 0,
                carry_enabled: false,
                liquidation_threshold: 9_000,
                oracle_sources: vec![],
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
                user: owner.pubkey(),
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
                total_shares: 0,
                vault_usdc_balance: 0,
                locked_for_financing: financing_amount,
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
            data: token_account_data(collateral_mint, vault_authority_pda, collateral_amount),
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
        vault_authority_pda,
        Account {
            lamports: 1_000_000,
            data: vec![],
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    CloseEarlyFixture {
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

async fn submit_close_early(
    program_test: ProgramTest,
    signer: &Keypair,
    receiver: Pubkey,
    fixture: &CloseEarlyFixture,
) -> Result<ProgramTestContext, BanksClientError> {
    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, signer).await;

    let accounts = financing_engine::accounts::CloseEarly {
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
        associated_token_program: spl_associated_token_account::id(),
        system_program: solana_sdk::system_program::id(),
        protocol_config: fixture.protocol_config_pda,
    };

    let ix = Instruction {
        program_id: financing_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: financing_engine::instruction::CloseEarly {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&signer.pubkey()),
        &[signer],
        context.last_blockhash,
    );

    context.banks_client.process_transaction(tx).await?;
    Ok(context)
}

struct LiquidationFixture {
    state_pda: Pubkey,
    position_counter_pda: Pubkey,
    protocol_config_pda: Pubkey,
    vault_authority_pda: Pubkey,
    collateral_mint: Pubkey,
    financed_mint: Pubkey,
    vault_collateral_ata: Pubkey,
    liquidator_collateral_ata: Pubkey,
    lp_vault_state: Pubkey,
    vault_financed_ata: Pubkey,
    liquidator_financed_ata: Pubkey,
    oracle_pda: Pubkey,
}

fn add_liquidation_accounts(
    program_test: &mut ProgramTest,
    owner: &Keypair,
    liquidator: &Keypair,
    financing_amount: u64,
    collateral_amount: u64,
    liquidation_threshold: u64,
    oracle_price: i64,
    last_update_slot: u64,
    protocol_paused: bool,
) -> LiquidationFixture {
    let admin = Keypair::new();
    let collateral_mint = Pubkey::new_unique();
    let financed_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[
            b"financing",
            owner.pubkey().as_ref(),
            collateral_mint.as_ref(),
        ],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", owner.pubkey().as_ref()],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) =
        Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id());
    let (vault_authority_pda, _) =
        Pubkey::find_program_address(&[b"vault_authority"], &financing_engine::id());
    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());
    let (oracle_pda, _) = Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id());

    let vault_collateral_ata = Pubkey::new_unique();
    let liquidator_collateral_ata = Pubkey::new_unique();
    let vault_financed_ata = Pubkey::new_unique();
    let liquidator_financed_ata = Pubkey::new_unique();

    program_test.add_account(
        protocol_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&ProtocolConfig {
                admin_authority: admin.pubkey(),
                protocol_paused,
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
                user_pubkey: owner.pubkey(),
                collateral_mint,
                collateral_amount,
                collateral_usd_value: 100_000_000,
                financing_amount,
                initial_ltv: 5_000,
                max_ltv: 8_000,
                term_start: 0,
                term_end: 0,
                fee_schedule: 0,
                carry_enabled: false,
                liquidation_threshold,
                oracle_sources: vec![],
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
                user: owner.pubkey(),
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
                total_shares: 0,
                vault_usdc_balance: 0,
                locked_for_financing: financing_amount,
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
            data: token_account_data(collateral_mint, vault_authority_pda, collateral_amount),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        liquidator_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, liquidator.pubkey(), 0),
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
        liquidator_financed_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(financed_mint, liquidator.pubkey(), financing_amount),
            owner: spl_token::id(),
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
                synthetic_twap: oracle_price,
                last_twap_window: 0,
                frozen_price: 0,
                frozen_slot: 0,
                last_update_slot,
                paused: false,
            }),
            owner: oracle_framework::id(),
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

    LiquidationFixture {
        state_pda,
        position_counter_pda,
        protocol_config_pda,
        vault_authority_pda,
        collateral_mint,
        financed_mint,
        vault_collateral_ata,
        liquidator_collateral_ata,
        lp_vault_state,
        vault_financed_ata,
        liquidator_financed_ata,
        oracle_pda,
    }
}

async fn submit_liquidate(
    context: &mut ProgramTestContext,
    liquidator: &Keypair,
    fixture: &LiquidationFixture,
) -> Result<(), BanksClientError> {
    let accounts = financing_engine::accounts::Liquidate {
        state: fixture.state_pda,
        collateral_mint: fixture.collateral_mint,
        vault_collateral_ata: fixture.vault_collateral_ata,
        liquidator_collateral_ata: fixture.liquidator_collateral_ata,
        vault_authority: fixture.vault_authority_pda,
        liquidator: liquidator.pubkey(),
        position_counter: fixture.position_counter_pda,
        token_program: spl_token::id(),
        lp_vault: fixture.lp_vault_state,
        financed_mint: fixture.financed_mint,
        vault_financed_ata: fixture.vault_financed_ata,
        liquidator_financed_ata: fixture.liquidator_financed_ata,
        lp_vault_program: lp_vault::id(),
        oracle: fixture.oracle_pda,
        protocol_config: fixture.protocol_config_pda,
    };

    let ix = Instruction {
        program_id: financing_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: financing_engine::instruction::Liquidate {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&liquidator.pubkey()),
        &[liquidator],
        context.last_blockhash,
    );

    context.banks_client.process_transaction(tx).await
}

struct ForceLiquidateFixture {
    state_pda: Pubkey,
    position_counter_pda: Pubkey,
    protocol_config_pda: Pubkey,
    vault_authority_pda: Pubkey,
    collateral_mint: Pubkey,
    vault_collateral_ata: Pubkey,
    protocol_collateral_ata: Pubkey,
    lp_vault_state: Pubkey,
}

fn add_force_liquidate_accounts(
    program_test: &mut ProgramTest,
    owner: &Keypair,
    authority: &Keypair,
    financing_amount: u64,
    collateral_amount: u64,
    protocol_paused: bool,
    lp_vault_authority: Pubkey,
    protocol_admin: Pubkey,
) -> ForceLiquidateFixture {
    let collateral_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[
            b"financing",
            owner.pubkey().as_ref(),
            collateral_mint.as_ref(),
        ],
        &financing_engine::id(),
    );
    let (position_counter_pda, _) = Pubkey::find_program_address(
        &[b"position_counter", owner.pubkey().as_ref()],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) =
        Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id());
    let (vault_authority_pda, _) =
        Pubkey::find_program_address(&[b"vault_authority"], &financing_engine::id());
    let (lp_vault_state, _) = Pubkey::find_program_address(&[b"vault"], &lp_vault::id());

    let vault_collateral_ata = Pubkey::new_unique();
    let protocol_collateral_ata = Pubkey::new_unique();

    program_test.add_account(
        protocol_config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&ProtocolConfig {
                admin_authority: protocol_admin,
                protocol_paused,
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
                user_pubkey: owner.pubkey(),
                collateral_mint,
                collateral_amount,
                collateral_usd_value: 100_000_000,
                financing_amount,
                initial_ltv: 5_000,
                max_ltv: 8_000,
                term_start: 0,
                term_end: 0,
                fee_schedule: 0,
                carry_enabled: false,
                liquidation_threshold: 9_000,
                oracle_sources: vec![],
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
                user: owner.pubkey(),
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
                total_shares: 0,
                vault_usdc_balance: 0,
                locked_for_financing: financing_amount,
                utilization: 0,
                authority: lp_vault_authority,
                paused: false,
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
            data: mint_data(protocol_admin),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    program_test.add_account(
        vault_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, vault_authority_pda, collateral_amount),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        protocol_collateral_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(collateral_mint, authority.pubkey(), 0),
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

    ForceLiquidateFixture {
        state_pda,
        position_counter_pda,
        protocol_config_pda,
        vault_authority_pda,
        collateral_mint,
        vault_collateral_ata,
        protocol_collateral_ata,
        lp_vault_state,
    }
}

async fn submit_force_liquidate(
    program_test: ProgramTest,
    authority: &Keypair,
    fixture: ForceLiquidateFixture,
    current_price: u64,
) -> Result<(), BanksClientError> {
    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, authority).await;

    let accounts = financing_engine::accounts::ForceLiquidate {
        state: fixture.state_pda,
        protocol_config: fixture.protocol_config_pda,
        collateral_mint: fixture.collateral_mint,
        vault_collateral_ata: fixture.vault_collateral_ata,
        protocol_collateral_ata: fixture.protocol_collateral_ata,
        vault_authority: fixture.vault_authority_pda,
        authority: authority.pubkey(),
        position_counter: fixture.position_counter_pda,
        token_program: spl_token::id(),
        lp_vault: fixture.lp_vault_state,
        lp_vault_program: lp_vault::id(),
    };

    let ix = Instruction {
        program_id: financing_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: financing_engine::instruction::ForceLiquidate { current_price }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&authority.pubkey()),
        &[authority],
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
        0,
        0,
        -1,
    );

    let result = submit_close_at_maturity(program_test, &bob, bob.pubkey(), &fixture).await;
    let err = result.expect_err("unauthorized close should fail");
    assert_financing_error(err, FinancingError::Unauthorized);
}

#[tokio::test]
async fn test_close_at_maturity_rejects_insufficient_repayment() {
    let mut program_test = setup_program_test();

    let alice = Keypair::new();

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        false,
        0,
        10_000,
        0,
        0,
        -1,
    );

    let result = submit_close_at_maturity(program_test, &alice, alice.pubkey(), &fixture).await;
    let err = result.expect_err("repayment should fail");
    assert_financing_error(err, FinancingError::InsufficientBalanceForClosure);
}

#[tokio::test]
async fn test_close_at_maturity_rejected_when_paused() {
    let mut program_test = setup_program_test();

    let alice = Keypair::new();

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        true,
        0,
        0,
        0,
        0,
        -1,
    );

    let result = submit_close_at_maturity(program_test, &alice, alice.pubkey(), &fixture).await;
    let err = result.expect_err("paused protocol should fail");
    assert_financing_error(err, FinancingError::ProtocolPaused);
}

#[tokio::test]
async fn test_initialize_financing_success() {
    let mut program_test = setup_program_test();
    let user = Keypair::new();
    let collateral_amount = 1_000_000;
    let financing_amount = common::setup::MIN_FINANCING_AMOUNT;

    let fixture = add_initialize_financing_accounts(
        &mut program_test,
        &user,
        collateral_amount,
        financing_amount,
        false,
        Some(0),
    );

    let mut context = submit_initialize_financing(
        program_test,
        &user,
        &fixture,
        collateral_amount,
        common::setup::MIN_COLLATERAL_USD,
        financing_amount,
        5_000,
        8_000,
        9_000,
        0,
        100,
    )
    .await
    .expect("initialize financing should succeed");

    let state_account = context
        .banks_client
        .get_account(fixture.state_pda)
        .await
        .unwrap()
        .expect("state account");
    let mut data_slice = state_account.data.as_slice();
    let state = FinancingState::try_deserialize(&mut data_slice).expect("deserialize state");
    assert_eq!(state.collateral_amount, collateral_amount);
    assert_eq!(state.financing_amount, financing_amount);
    assert_eq!(state.position_status, PositionStatus::Active);

    let counter_account = context
        .banks_client
        .get_account(fixture.position_counter_pda)
        .await
        .unwrap()
        .expect("position counter");
    let mut counter_slice = counter_account.data.as_slice();
    let counter =
        UserPositionCounter::try_deserialize(&mut counter_slice).expect("deserialize counter");
    assert_eq!(counter.open_positions, 1);

    let user_collateral = context
        .banks_client
        .get_account(fixture.user_collateral_ata)
        .await
        .unwrap()
        .expect("user collateral");
    let user_collateral = spl_token::state::Account::unpack(&user_collateral.data).expect("unpack");
    assert_eq!(user_collateral.amount, 0);

    let vault_collateral = context
        .banks_client
        .get_account(fixture.vault_collateral_ata)
        .await
        .unwrap()
        .expect("vault collateral");
    let vault_collateral =
        spl_token::state::Account::unpack(&vault_collateral.data).expect("unpack");
    assert_eq!(vault_collateral.amount, collateral_amount);

    let user_financed = context
        .banks_client
        .get_account(fixture.user_financed_ata)
        .await
        .unwrap()
        .expect("user financed");
    let user_financed = spl_token::state::Account::unpack(&user_financed.data).expect("unpack");
    assert_eq!(user_financed.amount, financing_amount);

    let vault_financed = context
        .banks_client
        .get_account(fixture.vault_financed_ata)
        .await
        .unwrap()
        .expect("vault financed");
    let vault_financed = spl_token::state::Account::unpack(&vault_financed.data).expect("unpack");
    assert_eq!(vault_financed.amount, 0);
}

#[tokio::test]
async fn test_initialize_financing_below_minimum() {
    let mut program_test = setup_program_test();
    let user = Keypair::new();
    let collateral_amount = 1_000_000;
    let financing_amount = common::setup::MIN_FINANCING_AMOUNT;

    let fixture = add_initialize_financing_accounts(
        &mut program_test,
        &user,
        collateral_amount,
        financing_amount,
        false,
        Some(0),
    );

    let result = submit_initialize_financing(
        program_test,
        &user,
        &fixture,
        collateral_amount,
        common::setup::MIN_COLLATERAL_USD - 1,
        financing_amount,
        5_000,
        8_000,
        9_000,
        0,
        100,
    )
    .await;
    let err = result.expect_err("position below minimum should fail");
    assert_financing_error(err, FinancingError::PositionTooSmall);
}

#[tokio::test]
async fn test_initialize_financing_ltv_ordering() {
    let mut program_test = setup_program_test();
    let user = Keypair::new();
    let collateral_amount = 1_000_000;
    let financing_amount = common::setup::MIN_FINANCING_AMOUNT;

    let fixture = add_initialize_financing_accounts(
        &mut program_test,
        &user,
        collateral_amount,
        financing_amount,
        false,
        Some(0),
    );

    let result = submit_initialize_financing(
        program_test,
        &user,
        &fixture,
        collateral_amount,
        common::setup::MIN_COLLATERAL_USD,
        financing_amount,
        9_000,
        8_000,
        9_000,
        0,
        100,
    )
    .await;
    let err = result.expect_err("ltv ordering should fail");
    assert_financing_error(err, FinancingError::InvalidLtvOrdering);
}

#[tokio::test]
async fn test_initialize_financing_position_limit() {
    let mut program_test = setup_program_test();
    let user = Keypair::new();
    let collateral_amount = 1_000_000;
    let financing_amount = common::setup::MIN_FINANCING_AMOUNT;

    let fixture = add_initialize_financing_accounts(
        &mut program_test,
        &user,
        collateral_amount,
        financing_amount,
        false,
        Some(UserPositionCounter::MAX_POSITIONS),
    );

    let result = submit_initialize_financing(
        program_test,
        &user,
        &fixture,
        collateral_amount,
        common::setup::MIN_COLLATERAL_USD,
        financing_amount,
        5_000,
        8_000,
        9_000,
        0,
        100,
    )
    .await;
    let err = result.expect_err("position limit should fail");
    assert_financing_error(err, FinancingError::TooManyPositions);
}

#[tokio::test]
async fn test_initialize_financing_while_paused() {
    let mut program_test = setup_program_test();
    let user = Keypair::new();
    let collateral_amount = 1_000_000;
    let financing_amount = common::setup::MIN_FINANCING_AMOUNT;

    let fixture = add_initialize_financing_accounts(
        &mut program_test,
        &user,
        collateral_amount,
        financing_amount,
        true,
        Some(0),
    );

    let result = submit_initialize_financing(
        program_test,
        &user,
        &fixture,
        collateral_amount,
        common::setup::MIN_COLLATERAL_USD,
        financing_amount,
        5_000,
        8_000,
        9_000,
        0,
        100,
    )
    .await;
    let err = result.expect_err("paused protocol should fail");
    assert_financing_error(err, FinancingError::ProtocolPaused);
}

#[tokio::test]
async fn test_close_at_maturity_success() {
    let mut program_test = setup_program_test();
    let alice = Keypair::new();
    let collateral_amount = 5_000;
    let financing_amount = 10_000;
    let fee_schedule = 500;
    let user_financed_amount = financing_amount + fee_schedule;

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        false,
        user_financed_amount,
        financing_amount,
        collateral_amount,
        fee_schedule,
        -1,
    );

    let mut context = submit_close_at_maturity(program_test, &alice, alice.pubkey(), &fixture)
        .await
        .expect("close at maturity should succeed");

    let user_collateral = context
        .banks_client
        .get_account(fixture.user_collateral_ata)
        .await
        .unwrap()
        .expect("user collateral");
    let user_collateral = spl_token::state::Account::unpack(&user_collateral.data).expect("unpack");
    assert_eq!(user_collateral.amount, collateral_amount);

    let vault_collateral = context
        .banks_client
        .get_account(fixture.vault_collateral_ata)
        .await
        .unwrap()
        .expect("vault collateral");
    let vault_collateral =
        spl_token::state::Account::unpack(&vault_collateral.data).expect("unpack");
    assert_eq!(vault_collateral.amount, 0);

    let user_financed = context
        .banks_client
        .get_account(fixture.user_financed_ata)
        .await
        .unwrap()
        .expect("user financed");
    let user_financed = spl_token::state::Account::unpack(&user_financed.data).expect("unpack");
    assert_eq!(user_financed.amount, 0);

    let vault_financed = context
        .banks_client
        .get_account(fixture.vault_financed_ata)
        .await
        .unwrap()
        .expect("vault financed");
    let vault_financed = spl_token::state::Account::unpack(&vault_financed.data).expect("unpack");
    assert_eq!(vault_financed.amount, user_financed_amount);

    let counter_account = context
        .banks_client
        .get_account(fixture.position_counter_pda)
        .await
        .unwrap()
        .expect("position counter");
    let mut counter_slice = counter_account.data.as_slice();
    let counter =
        UserPositionCounter::try_deserialize(&mut counter_slice).expect("deserialize counter");
    assert_eq!(counter.open_positions, 0);
}

#[tokio::test]
async fn test_close_at_maturity_with_outstanding_debt() {
    let mut program_test = setup_program_test();
    let alice = Keypair::new();
    let collateral_amount = 5_000;
    let financing_amount = 10_000;
    let fee_schedule = 500;

    let fixture = add_close_at_maturity_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        false,
        financing_amount,
        financing_amount,
        collateral_amount,
        fee_schedule,
        -1,
    );

    let result = submit_close_at_maturity(program_test, &alice, alice.pubkey(), &fixture).await;
    let err = result.expect_err("outstanding debt should fail");
    assert_financing_error(err, FinancingError::InsufficientBalanceForClosure);
}

#[tokio::test]
async fn test_close_early_fee_calculation() {
    let mut program_test = setup_program_test();
    let alice = Keypair::new();
    let collateral_amount = 10_000;
    let financing_amount = 1_000;

    let fixture = add_close_early_accounts(
        &mut program_test,
        &alice,
        alice.pubkey(),
        false,
        financing_amount,
        financing_amount,
        collateral_amount,
        1_000_000,
    );

    let mut context = submit_close_early(program_test, &alice, alice.pubkey(), &fixture)
        .await
        .expect("close early should succeed");

    let expected_fee = collateral_amount * 50 / 10_000;
    let expected_return = collateral_amount - expected_fee;

    let user_collateral = context
        .banks_client
        .get_account(fixture.user_collateral_ata)
        .await
        .unwrap()
        .expect("user collateral");
    let user_collateral = spl_token::state::Account::unpack(&user_collateral.data).expect("unpack");
    assert_eq!(user_collateral.amount, expected_return);

    let vault_collateral = context
        .banks_client
        .get_account(fixture.vault_collateral_ata)
        .await
        .unwrap()
        .expect("vault collateral");
    let vault_collateral =
        spl_token::state::Account::unpack(&vault_collateral.data).expect("unpack");
    assert_eq!(vault_collateral.amount, expected_fee);

    let counter_account = context
        .banks_client
        .get_account(fixture.position_counter_pda)
        .await
        .unwrap()
        .expect("position counter");
    let mut counter_slice = counter_account.data.as_slice();
    let counter =
        UserPositionCounter::try_deserialize(&mut counter_slice).expect("deserialize counter");
    assert_eq!(counter.open_positions, 0);
}

#[tokio::test]
async fn test_update_ltv_oracle_authorization() {
    let mut program_test = setup_program_test();
    let admin = Keypair::new();
    let unauthorized = Keypair::new();
    let user = Keypair::new();
    let collateral_mint = Pubkey::new_unique();

    let (state_pda, _) = Pubkey::find_program_address(
        &[
            b"financing",
            user.pubkey().as_ref(),
            collateral_mint.as_ref(),
        ],
        &financing_engine::id(),
    );
    let (protocol_config_pda, _) =
        Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id());

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
                collateral_amount: 1_000_000,
                collateral_usd_value: 100_000_000,
                financing_amount: 50_000_000,
                initial_ltv: 5_000,
                max_ltv: 8_000,
                term_start: 0,
                term_end: 100,
                fee_schedule: 0,
                carry_enabled: false,
                liquidation_threshold: 9_000,
                oracle_sources: vec![Pubkey::new_unique()],
                delegated_settlement_authority: Pubkey::default(),
                delegated_liquidation_authority: Pubkey::default(),
                position_status: PositionStatus::Active,
            }),
            owner: financing_engine::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &unauthorized).await;

    let accounts = financing_engine::accounts::UpdateLtv {
        state: state_pda,
        protocol_config: protocol_config_pda,
        authority: unauthorized.pubkey(),
    };

    let ix = Instruction {
        program_id: financing_engine::id(),
        accounts: accounts.to_account_metas(None),
        data: financing_engine::instruction::UpdateLtv {
            collateral_usd_value: 120_000_000,
        }
        .data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&unauthorized.pubkey()),
        &[&unauthorized],
        context.last_blockhash,
    );

    let result = context.banks_client.process_transaction(tx).await;
    let err = result.expect_err("unauthorized update should fail");
    assert_financing_error(err, FinancingError::Unauthorized);
}

#[tokio::test]
async fn test_liquidate_valid_threshold() {
    let mut program_test = setup_program_test();
    let owner = Keypair::new();
    let liquidator = Keypair::new();
    let financing_amount = 900_000;
    let collateral_amount = 1_000_000;

    let fixture = add_liquidation_accounts(
        &mut program_test,
        &owner,
        &liquidator,
        financing_amount,
        collateral_amount,
        9_000,
        100_000_000,
        0,
        false,
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &liquidator).await;

    submit_liquidate(&mut context, &liquidator, &fixture)
        .await
        .expect("liquidation should succeed");

    let liquidator_collateral = context
        .banks_client
        .get_account(fixture.liquidator_collateral_ata)
        .await
        .unwrap()
        .expect("liquidator collateral");
    let liquidator_collateral =
        spl_token::state::Account::unpack(&liquidator_collateral.data).expect("unpack");
    assert_eq!(liquidator_collateral.amount, collateral_amount);

    let vault_collateral = context
        .banks_client
        .get_account(fixture.vault_collateral_ata)
        .await
        .unwrap()
        .expect("vault collateral");
    let vault_collateral =
        spl_token::state::Account::unpack(&vault_collateral.data).expect("unpack");
    assert_eq!(vault_collateral.amount, 0);

    let liquidator_financed = context
        .banks_client
        .get_account(fixture.liquidator_financed_ata)
        .await
        .unwrap()
        .expect("liquidator financed");
    let liquidator_financed =
        spl_token::state::Account::unpack(&liquidator_financed.data).expect("unpack");
    assert_eq!(liquidator_financed.amount, 0);

    let vault_financed = context
        .banks_client
        .get_account(fixture.vault_financed_ata)
        .await
        .unwrap()
        .expect("vault financed");
    let vault_financed = spl_token::state::Account::unpack(&vault_financed.data).expect("unpack");
    assert_eq!(vault_financed.amount, financing_amount);

    let counter_account = context
        .banks_client
        .get_account(fixture.position_counter_pda)
        .await
        .unwrap()
        .expect("position counter");
    let mut counter_slice = counter_account.data.as_slice();
    let counter =
        UserPositionCounter::try_deserialize(&mut counter_slice).expect("deserialize counter");
    assert_eq!(counter.open_positions, 0);
}

#[tokio::test]
async fn test_liquidate_oracle_price_validation() {
    let mut program_test = setup_program_test();
    let owner = Keypair::new();
    let liquidator = Keypair::new();
    let financing_amount = 900_000;
    let collateral_amount = 1_000_000;

    let fixture = add_liquidation_accounts(
        &mut program_test,
        &owner,
        &liquidator,
        financing_amount,
        collateral_amount,
        9_000,
        100_000_000,
        0,
        false,
    );

    let mut context = program_test.start_with_context().await;
    fund_signer(&mut context, &liquidator).await;
    context.warp_to_slot(200).unwrap();
    context.last_blockhash = context.banks_client.get_latest_blockhash().await.unwrap();

    let result = submit_liquidate(&mut context, &liquidator, &fixture).await;
    let err = result.expect_err("stale oracle should fail");
    assert_financing_error(err, FinancingError::OraclePriceStale);
}

#[tokio::test]
async fn test_force_liquidate_admin_only() {
    let mut program_test = setup_program_test();
    let owner = Keypair::new();
    let authority = Keypair::new();
    let admin = Keypair::new();

    let fixture = add_force_liquidate_accounts(
        &mut program_test,
        &owner,
        &authority,
        900_000,
        1_000_000,
        false,
        Pubkey::new_unique(),
        admin.pubkey(),
    );

    let result = submit_force_liquidate(program_test, &authority, fixture, 100_000_000).await;
    let err = result.expect_err("unauthorized force liquidation should fail");
    assert_financing_error(err, FinancingError::Unauthorized);
}

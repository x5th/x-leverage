mod common;

use anchor_lang::prelude::{AccountDeserialize, AccountSerialize, Pubkey};
use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_spl::token::spl_token;
use common::setup::{mint_data, token_account_data};
use governance::{GovernanceConfig, GovernanceError, Proposal, VoteRecord};
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

fn governance_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    governance::entry(program_id, accounts, data)
}

fn add_governance_config(
    program_test: &mut ProgramTest,
    admin: Pubkey,
    quorum_votes: u64,
    voting_period: i64,
    timelock_delay: i64,
    paused: bool,
) -> Pubkey {
    let (config_pda, _) = Pubkey::find_program_address(&[b"governance_config"], &governance::id());
    let config = GovernanceConfig {
        quorum_votes,
        voting_period,
        timelock_delay,
        proposal_count: 0,
        admin_authority: admin,
        paused,
    };
    program_test.add_account(
        config_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&config),
            owner: governance::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    config_pda
}

fn add_proposal(program_test: &mut ProgramTest, proposal_pda: Pubkey, proposal: Proposal) {
    program_test.add_account(
        proposal_pda,
        Account {
            lamports: 1_000_000,
            data: serialize_anchor_account(&proposal),
            owner: governance::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
}

#[tokio::test]
async fn test_create_proposal_with_nonce() {
    let mut program_test = ProgramTest::new(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );

    let admin = Keypair::new();
    let creator = Keypair::new();
    let config_pda = add_governance_config(&mut program_test, admin.pubkey(), 1_000, 86_400, 172_800, false);

    let mut context = program_test.start_with_context().await;
    let fund_creator = system_instruction::transfer(
        &context.payer.pubkey(),
        &creator.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_creator],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let nonce = 7u64;
    let (proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &nonce.to_le_bytes()],
        &governance::id(),
    );
    let accounts = governance::accounts::CreateProposal {
        proposal: proposal_pda,
        governance_config: config_pda,
        creator: creator.pubkey(),
        system_program: system_program::id(),
    };
    let ix = Instruction {
        program_id: governance::id(),
        accounts: accounts.to_account_metas(None),
        data: governance::instruction::CreateProposal {
            proposal_nonce: nonce,
            title: "Proposal".to_string(),
            description: "Description".to_string(),
            eta: 0,
        }
        .data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&creator.pubkey()),
        &[&creator],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let config_account = context
        .banks_client
        .get_account(config_pda)
        .await
        .expect("fetch config")
        .expect("config exists");
    let mut config_data = config_account.data.as_slice();
    let config = GovernanceConfig::try_deserialize(&mut config_data).expect("deserialize config");
    assert_eq!(config.proposal_count, 1);

    let proposal_account = context
        .banks_client
        .get_account(proposal_pda)
        .await
        .expect("fetch proposal")
        .expect("proposal exists");
    let mut proposal_data = proposal_account.data.as_slice();
    let proposal = Proposal::try_deserialize(&mut proposal_data).expect("deserialize proposal");
    assert_eq!(proposal.nonce, nonce);
    assert_eq!(proposal.title, "Proposal");

    let duplicate_ix = Instruction {
        program_id: governance::id(),
        accounts: accounts.to_account_metas(None),
        data: governance::instruction::CreateProposal {
            proposal_nonce: nonce,
            title: "Duplicate".to_string(),
            description: "Duplicate".to_string(),
            eta: 0,
        }
        .data(),
    };
    let duplicate_tx = Transaction::new_signed_with_payer(
        &[duplicate_ix],
        Some(&creator.pubkey()),
        &[&creator],
        context.last_blockhash,
    );
    let err = context
        .banks_client
        .process_transaction(duplicate_tx)
        .await
        .expect_err("duplicate proposal should fail");
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::AccountAlreadyInitialized | InstructionError::AccountAlreadyInUse,
        )) => {}
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_vote_with_token_balance() {
    let mut program_test = ProgramTest::new(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );

    let admin = Keypair::new();
    let voter = Keypair::new();
    let config_pda = add_governance_config(&mut program_test, admin.pubkey(), 1_000, 86_400, 172_800, false);

    let nonce = 1u64;
    let creator = Keypair::new();
    let (proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &nonce.to_le_bytes()],
        &governance::id(),
    );
    add_proposal(
        &mut program_test,
        proposal_pda,
        Proposal {
            creator: creator.pubkey(),
            nonce,
            title: "Proposal".to_string(),
            description: "Description".to_string(),
            for_votes: 0,
            against_votes: 0,
            timelock_eta: 0,
            executed: false,
        },
    );

    let xgt_mint = Pubkey::new_unique();
    let voter_token_account = Pubkey::new_unique();
    let voter_balance = 2_000u64;
    program_test.add_account(
        xgt_mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(admin.pubkey()),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        voter_token_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(xgt_mint, voter.pubkey(), voter_balance),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    let zero_voter = Keypair::new();
    let zero_voter_ata = Pubkey::new_unique();
    program_test.add_account(
        zero_voter_ata,
        Account {
            lamports: 1_000_000,
            data: token_account_data(xgt_mint, zero_voter.pubkey(), 0),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let mut context = program_test.start_with_context().await;
    let fund_voter = system_instruction::transfer(
        &context.payer.pubkey(),
        &voter.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_voter],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let (vote_record_pda, _) = Pubkey::find_program_address(
        &[b"vote", proposal_pda.as_ref(), voter.pubkey().as_ref()],
        &governance::id(),
    );
    let accounts = governance::accounts::Vote {
        proposal: proposal_pda,
        vote_record: vote_record_pda,
        voter: voter.pubkey(),
        user_xgt_account: voter_token_account,
        xgt_mint,
        system_program: system_program::id(),
        governance_config: config_pda,
    };
    let ix = Instruction {
        program_id: governance::id(),
        accounts: accounts.to_account_metas(None),
        data: governance::instruction::Vote { support: true }.data(),
    };
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&voter.pubkey()),
        &[&voter],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let proposal_account = context
        .banks_client
        .get_account(proposal_pda)
        .await
        .expect("fetch proposal")
        .expect("proposal exists");
    let mut proposal_data = proposal_account.data.as_slice();
    let proposal = Proposal::try_deserialize(&mut proposal_data).expect("deserialize proposal");
    assert_eq!(proposal.for_votes, voter_balance);

    let vote_record_account = context
        .banks_client
        .get_account(vote_record_pda)
        .await
        .expect("fetch vote record")
        .expect("vote record exists");
    let mut vote_record_data = vote_record_account.data.as_slice();
    let vote_record = VoteRecord::try_deserialize(&mut vote_record_data).expect("deserialize vote record");
    assert!(vote_record.has_voted);
    assert_eq!(vote_record.weight, voter_balance);

    let fund_zero_voter = system_instruction::transfer(
        &context.payer.pubkey(),
        &zero_voter.pubkey(),
        1_000_000_000,
    );
    let fund_zero_tx = Transaction::new_signed_with_payer(
        &[fund_zero_voter],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_zero_tx).await.unwrap();

    let (zero_vote_record_pda, _) = Pubkey::find_program_address(
        &[b"vote", proposal_pda.as_ref(), zero_voter.pubkey().as_ref()],
        &governance::id(),
    );
    let zero_accounts = governance::accounts::Vote {
        proposal: proposal_pda,
        vote_record: zero_vote_record_pda,
        voter: zero_voter.pubkey(),
        user_xgt_account: zero_voter_ata,
        xgt_mint,
        system_program: system_program::id(),
        governance_config: config_pda,
    };
    let zero_ix = Instruction {
        program_id: governance::id(),
        accounts: zero_accounts.to_account_metas(None),
        data: governance::instruction::Vote { support: true }.data(),
    };
    let zero_tx = Transaction::new_signed_with_payer(
        &[zero_ix],
        Some(&zero_voter.pubkey()),
        &[&zero_voter],
        context.last_blockhash,
    );
    let err = context
        .banks_client
        .process_transaction(zero_tx)
        .await
        .expect_err("zero balance vote should fail");
    let expected = u32::from(GovernanceError::NoVotingPower);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn test_queue_execution_timelock() {
    let mut program_test = ProgramTest::new(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );

    let (config_pda, _) = Pubkey::find_program_address(&[b"governance_config"], &governance::id());

    let mut context = program_test.start_with_context().await;
    let short_ix = Instruction {
        program_id: governance::id(),
        accounts: governance::accounts::InitializeGovernance {
            governance_config: config_pda,
            payer: context.payer.pubkey(),
            system_program: system_program::id(),
        }
        .to_account_metas(None),
        data: governance::instruction::InitializeGovernance {
            quorum_votes: 1_000,
            voting_period: 86_400,
            timelock_delay: 1_000,
            admin_authority: context.payer.pubkey(),
        }
        .data(),
    };
    let short_tx = Transaction::new_signed_with_payer(
        &[short_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    let err = context
        .banks_client
        .process_transaction(short_tx)
        .await
        .expect_err("short timelock should fail");
    let expected = u32::from(GovernanceError::TimelockTooShort);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let ok_ix = Instruction {
        program_id: governance::id(),
        accounts: governance::accounts::InitializeGovernance {
            governance_config: config_pda,
            payer: context.payer.pubkey(),
            system_program: system_program::id(),
        }
        .to_account_metas(None),
        data: governance::instruction::InitializeGovernance {
            quorum_votes: 1_000,
            voting_period: 86_400,
            timelock_delay: 172_800,
            admin_authority: context.payer.pubkey(),
        }
        .data(),
    };
    let ok_tx = Transaction::new_signed_with_payer(
        &[ok_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(ok_tx).await.unwrap();

    let config_account = context
        .banks_client
        .get_account(config_pda)
        .await
        .expect("fetch config")
        .expect("config exists");
    let mut config_data = config_account.data.as_slice();
    let config = GovernanceConfig::try_deserialize(&mut config_data).expect("deserialize config");
    assert_eq!(config.timelock_delay, 172_800);
    assert!(!config.paused);
}

#[tokio::test]
async fn test_execute_proposal_quorum() {
    let mut program_test = ProgramTest::new(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );

    let admin = Keypair::new();
    let executor = Keypair::new();
    let config_pda = add_governance_config(&mut program_test, admin.pubkey(), 1_000, 86_400, 172_800, false);

    let creator = Keypair::new();
    let low_nonce = 1u64;
    let (low_proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &low_nonce.to_le_bytes()],
        &governance::id(),
    );
    add_proposal(
        &mut program_test,
        low_proposal_pda,
        Proposal {
            creator: creator.pubkey(),
            nonce: low_nonce,
            title: "Low".to_string(),
            description: "Low".to_string(),
            for_votes: 500,
            against_votes: 0,
            timelock_eta: 0,
            executed: false,
        },
    );

    let high_nonce = 2u64;
    let (high_proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &high_nonce.to_le_bytes()],
        &governance::id(),
    );
    add_proposal(
        &mut program_test,
        high_proposal_pda,
        Proposal {
            creator: creator.pubkey(),
            nonce: high_nonce,
            title: "High".to_string(),
            description: "High".to_string(),
            for_votes: 1_500,
            against_votes: 100,
            timelock_eta: 0,
            executed: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    let fund_executor = system_instruction::transfer(
        &context.payer.pubkey(),
        &executor.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_executor],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let low_accounts = governance::accounts::ExecuteProposal {
        proposal: low_proposal_pda,
        governance_config: config_pda,
        executor: executor.pubkey(),
    };
    let low_ix = Instruction {
        program_id: governance::id(),
        accounts: low_accounts.to_account_metas(None),
        data: governance::instruction::Execute {}.data(),
    };
    let low_tx = Transaction::new_signed_with_payer(
        &[low_ix],
        Some(&executor.pubkey()),
        &[&executor],
        context.last_blockhash,
    );
    let err = context
        .banks_client
        .process_transaction(low_tx)
        .await
        .expect_err("quorum failure should error");
    let expected = u32::from(GovernanceError::QuorumNotReached);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let high_accounts = governance::accounts::ExecuteProposal {
        proposal: high_proposal_pda,
        governance_config: config_pda,
        executor: executor.pubkey(),
    };
    let high_ix = Instruction {
        program_id: governance::id(),
        accounts: high_accounts.to_account_metas(None),
        data: governance::instruction::Execute {}.data(),
    };
    let high_tx = Transaction::new_signed_with_payer(
        &[high_ix],
        Some(&executor.pubkey()),
        &[&executor],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(high_tx).await.unwrap();

    let proposal_account = context
        .banks_client
        .get_account(high_proposal_pda)
        .await
        .expect("fetch proposal")
        .expect("proposal exists");
    let mut proposal_data = proposal_account.data.as_slice();
    let proposal = Proposal::try_deserialize(&mut proposal_data).expect("deserialize proposal");
    assert!(proposal.executed);
}

#[tokio::test]
async fn test_execute_proposal_authorization() {
    let mut program_test = ProgramTest::new(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );

    let admin = Keypair::new();
    let executor = Keypair::new();
    let config_pda = add_governance_config(&mut program_test, admin.pubkey(), 1_000, 86_400, 172_800, false);

    let creator = Keypair::new();
    let nonce = 9u64;
    let (proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &nonce.to_le_bytes()],
        &governance::id(),
    );
    add_proposal(
        &mut program_test,
        proposal_pda,
        Proposal {
            creator: creator.pubkey(),
            nonce,
            title: "Auth".to_string(),
            description: "Auth".to_string(),
            for_votes: 1_500,
            against_votes: 0,
            timelock_eta: 0,
            executed: false,
        },
    );

    let mut context = program_test.start_with_context().await;
    let fund_executor = system_instruction::transfer(
        &context.payer.pubkey(),
        &executor.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_executor],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let accounts = governance::accounts::ExecuteProposal {
        proposal: proposal_pda,
        governance_config: config_pda,
        executor: executor.pubkey(),
    };
    let ix = Instruction {
        program_id: governance::id(),
        accounts: accounts.to_account_metas(None),
        data: governance::instruction::Execute {}.data(),
    };
    let unsigned_tx = Transaction::new_signed_with_payer(
        &[ix.clone()],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    let err = context
        .banks_client
        .process_transaction(unsigned_tx)
        .await
        .expect_err("missing executor signature should fail");
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::MissingRequiredSignature,
        )) => {}
        other => panic!("unexpected error: {other:?}"),
    }

    let signed_tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(signed_tx).await.unwrap();

    let proposal_account = context
        .banks_client
        .get_account(proposal_pda)
        .await
        .expect("fetch proposal")
        .expect("proposal exists");
    let mut proposal_data = proposal_account.data.as_slice();
    let proposal = Proposal::try_deserialize(&mut proposal_data).expect("deserialize proposal");
    assert!(proposal.executed);
}

#[tokio::test]
async fn test_pause_governance_operations() {
    let mut program_test = ProgramTest::new(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );

    let admin = Keypair::new();
    let attacker = Keypair::new();
    let creator = Keypair::new();
    let config_pda = add_governance_config(&mut program_test, admin.pubkey(), 1_000, 86_400, 172_800, false);

    let mut context = program_test.start_with_context().await;
    let fund_admin = system_instruction::transfer(
        &context.payer.pubkey(),
        &admin.pubkey(),
        1_000_000_000,
    );
    let fund_attacker = system_instruction::transfer(
        &context.payer.pubkey(),
        &attacker.pubkey(),
        1_000_000_000,
    );
    let fund_creator = system_instruction::transfer(
        &context.payer.pubkey(),
        &creator.pubkey(),
        1_000_000_000,
    );
    let fund_tx = Transaction::new_signed_with_payer(
        &[fund_admin, fund_attacker, fund_creator],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(fund_tx).await.unwrap();

    let attacker_accounts = governance::accounts::AdminGovernanceAction {
        governance_config: config_pda,
        admin_authority: attacker.pubkey(),
    };
    let attacker_ix = Instruction {
        program_id: governance::id(),
        accounts: attacker_accounts.to_account_metas(None),
        data: governance::instruction::PauseGovernance {}.data(),
    };
    let attacker_tx = Transaction::new_signed_with_payer(
        &[attacker_ix],
        Some(&attacker.pubkey()),
        &[&attacker],
        context.last_blockhash,
    );
    let err = context
        .banks_client
        .process_transaction(attacker_tx)
        .await
        .expect_err("unauthorized pause should fail");
    let expected = u32::from(GovernanceError::Unauthorized);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let admin_accounts = governance::accounts::AdminGovernanceAction {
        governance_config: config_pda,
        admin_authority: admin.pubkey(),
    };
    let admin_ix = Instruction {
        program_id: governance::id(),
        accounts: admin_accounts.to_account_metas(None),
        data: governance::instruction::PauseGovernance {}.data(),
    };
    let admin_tx = Transaction::new_signed_with_payer(
        &[admin_ix],
        Some(&admin.pubkey()),
        &[&admin],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(admin_tx).await.unwrap();

    let config_account = context
        .banks_client
        .get_account(config_pda)
        .await
        .expect("fetch config")
        .expect("config exists");
    let mut config_data = config_account.data.as_slice();
    let config = GovernanceConfig::try_deserialize(&mut config_data).expect("deserialize config");
    assert!(config.paused);

    let nonce = 3u64;
    let (proposal_pda, _) = Pubkey::find_program_address(
        &[b"proposal", creator.pubkey().as_ref(), &nonce.to_le_bytes()],
        &governance::id(),
    );
    let create_accounts = governance::accounts::CreateProposal {
        proposal: proposal_pda,
        governance_config: config_pda,
        creator: creator.pubkey(),
        system_program: system_program::id(),
    };
    let create_ix = Instruction {
        program_id: governance::id(),
        accounts: create_accounts.to_account_metas(None),
        data: governance::instruction::CreateProposal {
            proposal_nonce: nonce,
            title: "Paused".to_string(),
            description: "Paused".to_string(),
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
    let err = context
        .banks_client
        .process_transaction(create_tx)
        .await
        .expect_err("paused governance should block proposal creation");
    let expected = u32::from(GovernanceError::GovernancePaused);
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(
            _,
            InstructionError::Custom(code),
        )) => {
            assert_eq!(code, expected);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

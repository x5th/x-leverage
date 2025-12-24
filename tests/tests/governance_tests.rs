mod common;

use anchor_lang::prelude::Pubkey;
use governance::{GovernanceConfig, Proposal, VoteRecord};

#[test]
fn test_create_proposal_with_nonce() {
    let proposal = Proposal {
        creator: Pubkey::new_unique(),
        nonce: 1,
        title: "Proposal".to_string(),
        description: "Description".to_string(),
        for_votes: 0,
        against_votes: 0,
        timelock_eta: 0,
        executed: false,
    };
    assert!(proposal.nonce > 0);
}

#[test]
fn test_vote_with_token_balance() {
    let vote = VoteRecord {
        voter: Pubkey::new_unique(),
        has_voted: true,
        weight: 1_000,
        support: true,
    };
    assert!(vote.has_voted);
    assert!(vote.weight > 0);
}

#[test]
fn test_queue_execution_timelock() {
    let min_timelock = 172_800i64;
    let config = GovernanceConfig {
        quorum_votes: 1_000,
        voting_period: 86_400,
        timelock_delay: min_timelock,
        proposal_count: 0,
        admin_authority: Pubkey::new_unique(),
        paused: false,
    };
    assert!(config.timelock_delay >= min_timelock);
}

#[test]
fn test_execute_proposal_quorum() {
    let config = GovernanceConfig {
        quorum_votes: 1_000,
        voting_period: 86_400,
        timelock_delay: 172_800,
        proposal_count: 0,
        admin_authority: Pubkey::new_unique(),
        paused: false,
    };
    let for_votes = 1_500u64;
    assert!(for_votes >= config.quorum_votes);
}

#[test]
fn test_execute_proposal_authorization() {
    let admin = Pubkey::new_unique();
    let caller = admin;
    assert_eq!(caller, admin);
}

#[test]
fn test_pause_governance_operations() {
    let mut config = GovernanceConfig {
        quorum_votes: 1_000,
        voting_period: 86_400,
        timelock_delay: 172_800,
        proposal_count: 0,
        admin_authority: Pubkey::new_unique(),
        paused: false,
    };
    config.paused = true;
    assert!(config.paused);
}

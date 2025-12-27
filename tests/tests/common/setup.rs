use anchor_lang::prelude::Pubkey;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::spl_token;
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program_option::COption;
use solana_program_pack::Pack;
use solana_program_test::ProgramTest;
use solana_sdk::account::Account;
use solana_sdk::signature::Keypair;

pub const MIN_COLLATERAL_USD: u64 = 100_000_000; // $100 (8 decimals)
pub const MIN_FINANCING_AMOUNT: u64 = 50_000_000; // $50 (6 decimals)

fn financing_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    financing_engine::entry(program_id, accounts, data)
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

fn settlement_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    settlement_engine::entry(program_id, accounts, data)
}

fn treasury_engine_processor<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    data: &'d [u8],
) -> ProgramResult {
    let accounts: &[AccountInfo<'_>] = unsafe { std::mem::transmute(accounts) };
    treasury_engine::entry(program_id, accounts, data)
}

pub fn program_test_with_financing_engine() -> ProgramTest {
    ProgramTest::new(
        "financing_engine",
        financing_engine::id(),
        solana_program_test::processor!(financing_engine_processor),
    )
}

pub fn add_financing_engine_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "financing_engine",
        financing_engine::id(),
        solana_program_test::processor!(financing_engine_processor),
    );
}

pub fn add_lp_vault_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "lp_vault",
        lp_vault::id(),
        solana_program_test::processor!(lp_vault_processor),
    );
}

pub fn add_oracle_framework_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "oracle_framework",
        oracle_framework::id(),
        solana_program_test::processor!(oracle_framework_processor),
    );
}

pub fn add_governance_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "governance",
        governance::id(),
        solana_program_test::processor!(governance_processor),
    );
}

pub fn add_liquidation_engine_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "liquidation_engine",
        liquidation_engine::id(),
        solana_program_test::processor!(liquidation_engine_processor),
    );
}

pub fn add_treasury_engine_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "treasury_engine",
        treasury_engine::id(),
        solana_program_test::processor!(treasury_engine_processor),
    );
}

pub fn add_settlement_engine_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "settlement_engine",
        settlement_engine::id(),
        solana_program_test::processor!(settlement_engine_processor),
    );
}

pub fn add_spl_token_program(program_test: &mut ProgramTest) {
    program_test.add_program(
        "spl_token",
        spl_token::id(),
        solana_program_test::processor!(spl_token::processor::Processor::process),
    );
}

pub fn bootstrap_program_test(program_test: &mut ProgramTest) {
    add_lp_vault_program(program_test);
    add_oracle_framework_program(program_test);
    add_governance_program(program_test);
    add_liquidation_engine_program(program_test);
    add_treasury_engine_program(program_test);
    add_settlement_engine_program(program_test);
    add_spl_token_program(program_test);
}

pub fn token_account_data(mint: Pubkey, owner: Pubkey, amount: u64) -> Vec<u8> {
    let token_account = spl_token::state::Account {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: spl_token::state::AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; spl_token::state::Account::LEN];
    spl_token::state::Account::pack(token_account, &mut data).expect("pack token account");
    data
}

pub fn mint_data(mint_authority: Pubkey) -> Vec<u8> {
    let mint = spl_token::state::Mint {
        mint_authority: COption::Some(mint_authority),
        supply: 0,
        decimals: 6,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; spl_token::state::Mint::LEN];
    spl_token::state::Mint::pack(mint, &mut data).expect("pack mint");
    data
}

pub fn add_mint_account(program_test: &mut ProgramTest, mint: Pubkey, mint_authority: Pubkey) {
    program_test.add_account(
        mint,
        Account {
            lamports: 1_000_000,
            data: mint_data(mint_authority),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
}

pub fn add_token_account(
    program_test: &mut ProgramTest,
    token_account: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    program_test.add_account(
        token_account,
        Account {
            lamports: 1_000_000,
            data: token_account_data(mint, owner, amount),
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
}

pub fn add_mint_and_ata(
    program_test: &mut ProgramTest,
    mint_authority: Pubkey,
    owner: Pubkey,
    amount: u64,
) -> (Pubkey, Pubkey) {
    let mint = Pubkey::new_unique();
    let ata = get_associated_token_address(&owner, &mint);
    add_mint_account(program_test, mint, mint_authority);
    add_token_account(program_test, ata, mint, owner, amount);
    (mint, ata)
}

pub fn deterministic_pubkey(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

pub fn deterministic_keypair(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32]).expect("deterministic keypair")
}

pub fn oracle_sources() -> Vec<Pubkey> {
    vec![deterministic_pubkey(42), deterministic_pubkey(43)]
}

pub fn sample_protocol_admin() -> Pubkey {
    deterministic_pubkey(1)
}

pub fn sample_user() -> Pubkey {
    deterministic_pubkey(2)
}

pub fn sample_admin_keypair() -> Keypair {
    deterministic_keypair(1)
}

pub fn sample_user_keypair() -> Keypair {
    deterministic_keypair(2)
}

pub fn financing_state_pda(user: Pubkey, position_index: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"financing", user.as_ref(), &position_index.to_le_bytes()],
        &financing_engine::id(),
    )
}

pub fn financing_position_counter_pda(user: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"position_counter", user.as_ref()], &financing_engine::id())
}

pub fn financing_protocol_config_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"protocol_config"], &financing_engine::id())
}

pub fn financing_vault_authority_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault_authority"], &financing_engine::id())
}

pub fn lp_vault_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault"], &lp_vault::id())
}

pub fn oracle_framework_oracle_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"oracle"], &oracle_framework::id())
}

pub fn governance_config_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"governance_config"], &governance::id())
}

pub fn governance_proposal_pda(creator: Pubkey, nonce: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"proposal", creator.as_ref(), &nonce.to_le_bytes()],
        &governance::id(),
    )
}

pub fn governance_vote_pda(proposal: Pubkey, voter: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"vote", proposal.as_ref(), voter.as_ref()],
        &governance::id(),
    )
}

pub fn liquidation_state_pda(owner: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"liquidation", owner.as_ref()], &liquidation_engine::id())
}

pub fn treasury_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"treasury"], &treasury_engine::id())
}

pub fn settlement_config_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"settlement_config"], &settlement_engine::id())
}

pub fn settlement_pda(authority: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"settlement", authority.as_ref()], &settlement_engine::id())
}

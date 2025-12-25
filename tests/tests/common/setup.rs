use anchor_lang::prelude::Pubkey;
use anchor_spl::token::spl_token;
use solana_program_option::COption;
use solana_program_pack::Pack;

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

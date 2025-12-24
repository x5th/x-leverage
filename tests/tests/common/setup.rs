use anchor_lang::prelude::Pubkey;

pub const MIN_COLLATERAL_USD: u64 = 100_000_000; // $100 (8 decimals)
pub const MIN_FINANCING_AMOUNT: u64 = 50_000_000; // $50 (6 decimals)

pub fn deterministic_pubkey(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
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

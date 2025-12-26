use anchor_lang::prelude::*;

declare_id!("8criri7uvtARSwA6GpNSbQjxfAsGAx5raVUQSg2aHcS9");

#[program]
pub mod wrapping_vault {
    use super::*;

    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}

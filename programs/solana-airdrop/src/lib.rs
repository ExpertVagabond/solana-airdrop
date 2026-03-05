use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::{hash as sol_hash, hashv as sol_hashv};
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("CNcG4AK4uUXsqAjKQiFk5i9zU75MdHmgdJDXa5cCgYDH");

#[program]
pub mod solana_airdrop {
    use super::*;

    pub fn create_airdrop(ctx: Context<CreateAirdrop>, amount_per_claim: u64, max_claims: u64, merkle_root: [u8; 32]) -> Result<()> {
        let airdrop = &mut ctx.accounts.airdrop;
        airdrop.authority = ctx.accounts.authority.key();
        airdrop.mint = ctx.accounts.mint.key();
        airdrop.amount_per_claim = amount_per_claim;
        airdrop.max_claims = max_claims;
        airdrop.total_claimed = 0;
        airdrop.merkle_root = merkle_root;
        airdrop.active = true;
        airdrop.bump = ctx.bumps.airdrop;
        Ok(())
    }

    pub fn fund_airdrop(ctx: Context<FundAirdrop>, amount: u64) -> Result<()> {
        token::transfer(CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.authority_token_account.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            },
        ), amount)?;
        Ok(())
    }

    pub fn claim(ctx: Context<Claim>, proof: Vec<[u8; 32]>) -> Result<()> {
        let airdrop = &ctx.accounts.airdrop;
        require!(airdrop.active, AirdropError::NotActive);
        require!(airdrop.total_claimed < airdrop.max_claims, AirdropError::MaxClaimsReached);

        // Verify merkle proof
        let leaf = sol_hash(ctx.accounts.claimer.key().as_ref());
        let mut computed = leaf.to_bytes();
        for node in proof.iter() {
            if computed <= *node {
                computed = sol_hashv(&[&computed, node]).to_bytes();
            } else {
                computed = sol_hashv(&[node, &computed]).to_bytes();
            }
        }
        require!(computed == airdrop.merkle_root, AirdropError::InvalidProof);

        let authority_key = airdrop.authority;
        let mint_key = airdrop.mint;
        let bump = airdrop.bump;
        let seeds: &[&[u8]] = &[b"airdrop", authority_key.as_ref(), mint_key.as_ref(), &[bump]];

        token::transfer(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.claimer_token_account.to_account_info(),
                authority: ctx.accounts.airdrop.to_account_info(),
            },
            &[seeds],
        ), airdrop.amount_per_claim)?;

        let claim_record = &mut ctx.accounts.claim_record;
        claim_record.airdrop = airdrop.key();
        claim_record.claimer = ctx.accounts.claimer.key();
        claim_record.amount = airdrop.amount_per_claim;
        claim_record.claimed_at = Clock::get()?.unix_timestamp;
        claim_record.bump = ctx.bumps.claim_record;

        let airdrop = &mut ctx.accounts.airdrop;
        airdrop.total_claimed = airdrop.total_claimed.checked_add(1).ok_or(AirdropError::Overflow)?;
        Ok(())
    }

    pub fn close_airdrop(ctx: Context<CloseAirdrop>) -> Result<()> {
        let airdrop = &mut ctx.accounts.airdrop;
        airdrop.active = false;

        // Return remaining tokens to authority
        let remaining = ctx.accounts.vault.amount;
        if remaining > 0 {
            let authority_key = airdrop.authority;
            let mint_key = airdrop.mint;
            let bump = airdrop.bump;
            let seeds: &[&[u8]] = &[b"airdrop", authority_key.as_ref(), mint_key.as_ref(), &[bump]];

            token::transfer(CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ctx.accounts.authority_token_account.to_account_info(),
                    authority: ctx.accounts.airdrop.to_account_info(),
                },
                &[seeds],
            ), remaining)?;
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CreateAirdrop<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub mint: Account<'info, Mint>,
    #[account(init, payer = authority, space = 8 + Airdrop::INIT_SPACE,
        seeds = [b"airdrop", authority.key().as_ref(), mint.key().as_ref()], bump)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(init, payer = authority, token::mint = mint, token::authority = airdrop,
        seeds = [b"vault", airdrop.key().as_ref()], bump)]
    pub vault: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct FundAirdrop<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(seeds = [b"airdrop", airdrop.authority.as_ref(), airdrop.mint.as_ref()], bump = airdrop.bump, has_one = authority)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = airdrop.mint, token::authority = airdrop)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, constraint = authority_token_account.mint == airdrop.mint)]
    pub authority_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub claimer: Signer<'info>,
    #[account(mut, seeds = [b"airdrop", airdrop.authority.as_ref(), airdrop.mint.as_ref()], bump = airdrop.bump)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = airdrop.mint, token::authority = airdrop)]
    pub vault: Account<'info, TokenAccount>,
    #[account(init, payer = claimer, space = 8 + ClaimRecord::INIT_SPACE,
        seeds = [b"claim", airdrop.key().as_ref(), claimer.key().as_ref()], bump)]
    pub claim_record: Account<'info, ClaimRecord>,
    #[account(mut, constraint = claimer_token_account.mint == airdrop.mint)]
    pub claimer_token_account: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CloseAirdrop<'info> {
    pub authority: Signer<'info>,
    #[account(mut, has_one = authority)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = airdrop.mint, token::authority = airdrop)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, constraint = authority_token_account.mint == airdrop.mint)]
    pub authority_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[account]
#[derive(InitSpace)]
pub struct Airdrop {
    pub authority: Pubkey,
    pub mint: Pubkey,
    pub amount_per_claim: u64,
    pub max_claims: u64,
    pub total_claimed: u64,
    pub merkle_root: [u8; 32],
    pub active: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct ClaimRecord {
    pub airdrop: Pubkey,
    pub claimer: Pubkey,
    pub amount: u64,
    pub claimed_at: i64,
    pub bump: u8,
}

#[error_code]
pub enum AirdropError {
    #[msg("Airdrop not active")]
    NotActive,
    #[msg("Max claims reached")]
    MaxClaimsReached,
    #[msg("Invalid merkle proof")]
    InvalidProof,
    #[msg("Overflow")]
    Overflow,
}

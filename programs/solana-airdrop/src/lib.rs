use anchor_lang::prelude::*;
use solana_program::hash::{hash as sol_hash, hashv as sol_hashv};
use anchor_spl::token_interface::{Mint, TokenInterface, TokenAccount, TransferChecked, transfer_checked};
use solana_token_vesting::cpi::accounts::CreateVesting;
use solana_token_vesting::cpi::create_vesting;
use solana_token_vesting::program::SolanaTokenVesting;

declare_id!("FZPFToJZbiDnr74xotCMRJtCTHkpuUeaUvrgfZ7HfmMe");

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

        emit!(AirdropCreated {
            airdrop: airdrop.key(),
            authority: airdrop.authority,
            mint: airdrop.mint,
            merkle_root,
            max_claims,
        });

        Ok(())
    }

    pub fn fund_airdrop(ctx: Context<FundAirdrop>, amount: u64) -> Result<()> {
        let decimals = ctx.accounts.mint.decimals;
        transfer_checked(CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.authority_token_account.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
            },
        ), amount, decimals)?;
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

        let decimals = ctx.accounts.mint.decimals;
        transfer_checked(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.claimer_token_account.to_account_info(),
                authority: ctx.accounts.airdrop.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
            },
            &[seeds],
        ), airdrop.amount_per_claim, decimals)?;

        let claim_record = &mut ctx.accounts.claim_record;
        claim_record.airdrop = airdrop.key();
        claim_record.claimer = ctx.accounts.claimer.key();
        claim_record.amount = airdrop.amount_per_claim;
        claim_record.claimed_at = Clock::get()?.unix_timestamp;
        claim_record.bump = ctx.bumps.claim_record;

        let airdrop = &mut ctx.accounts.airdrop;
        airdrop.total_claimed = airdrop.total_claimed.checked_add(1).ok_or(AirdropError::Overflow)?;

        emit!(AirdropClaimed {
            airdrop: airdrop.key(),
            claimer: ctx.accounts.claimer.key(),
            amount: airdrop.amount_per_claim,
            total_claimed: airdrop.total_claimed,
        });

        Ok(())
    }

    /// Claim airdrop tokens into a vesting schedule via CPI to token-vesting.
    pub fn claim_with_vesting(
        ctx: Context<ClaimWithVesting>,
        proof: Vec<[u8; 32]>,
        start_ts: i64,
        cliff_ts: i64,
        end_ts: i64,
    ) -> Result<()> {
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

        let amount = airdrop.amount_per_claim;
        let authority_key = airdrop.authority;
        let mint_key = airdrop.mint;
        let bump = airdrop.bump;
        let seeds: &[&[u8]] = &[b"airdrop", authority_key.as_ref(), mint_key.as_ref(), &[bump]];

        // Transfer from airdrop vault to intermediary token account owned by airdrop PDA
        let decimals = ctx.accounts.mint.decimals;
        transfer_checked(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.airdrop_token_account.to_account_info(),
                authority: ctx.accounts.airdrop.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
            },
            &[seeds],
        ), amount, decimals)?;

        // CPI into token-vesting to create a vesting schedule
        let cpi_accounts = CreateVesting {
            authority: ctx.accounts.airdrop.to_account_info(),
            beneficiary: ctx.accounts.claimer.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            authority_token_account: ctx.accounts.airdrop_token_account.to_account_info(),
            vesting_schedule: ctx.accounts.vesting_schedule.to_account_info(),
            vault: ctx.accounts.vesting_vault.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };
        create_vesting(
            CpiContext::new_with_signer(ctx.accounts.vesting_program.to_account_info(), cpi_accounts, &[seeds]),
            amount, start_ts, cliff_ts, end_ts,
        )?;

        let claim_record = &mut ctx.accounts.claim_record;
        claim_record.airdrop = airdrop.key();
        claim_record.claimer = ctx.accounts.claimer.key();
        claim_record.amount = amount;
        claim_record.claimed_at = Clock::get()?.unix_timestamp;
        claim_record.bump = ctx.bumps.claim_record;

        let airdrop = &mut ctx.accounts.airdrop;
        airdrop.total_claimed = airdrop.total_claimed.checked_add(1).ok_or(AirdropError::Overflow)?;

        emit!(AirdropClaimedWithVesting {
            airdrop: airdrop.key(),
            claimer: ctx.accounts.claimer.key(),
            amount,
            total_claimed: airdrop.total_claimed,
            cliff_ts,
            end_ts,
        });

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

            let decimals = ctx.accounts.mint.decimals;
            transfer_checked(CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ctx.accounts.authority_token_account.to_account_info(),
                    authority: ctx.accounts.airdrop.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                },
                &[seeds],
            ), remaining, decimals)?;
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CreateAirdrop<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(init, payer = authority, space = 8 + Airdrop::INIT_SPACE,
        seeds = [b"airdrop", authority.key().as_ref(), mint.key().as_ref()], bump)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(init, payer = authority, token::mint = mint, token::authority = airdrop,
        seeds = [b"vault", airdrop.key().as_ref()], bump)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct FundAirdrop<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(seeds = [b"airdrop", airdrop.authority.as_ref(), airdrop.mint.as_ref()], bump = airdrop.bump, has_one = authority)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(address = airdrop.mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = mint, token::authority = airdrop)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, constraint = authority_token_account.mint == airdrop.mint)]
    pub authority_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub claimer: Signer<'info>,
    #[account(mut, seeds = [b"airdrop", airdrop.authority.as_ref(), airdrop.mint.as_ref()], bump = airdrop.bump)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(address = airdrop.mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = mint, token::authority = airdrop)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(init, payer = claimer, space = 8 + ClaimRecord::INIT_SPACE,
        seeds = [b"claim", airdrop.key().as_ref(), claimer.key().as_ref()], bump)]
    pub claim_record: Account<'info, ClaimRecord>,
    #[account(mut, constraint = claimer_token_account.mint == airdrop.mint)]
    pub claimer_token_account: InterfaceAccount<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct ClaimWithVesting<'info> {
    #[account(mut)]
    pub claimer: Signer<'info>,
    #[account(mut, seeds = [b"airdrop", airdrop.authority.as_ref(), airdrop.mint.as_ref()], bump = airdrop.bump)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(address = airdrop.mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = mint, token::authority = airdrop)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(init, payer = claimer, space = 8 + ClaimRecord::INIT_SPACE,
        seeds = [b"claim", airdrop.key().as_ref(), claimer.key().as_ref()], bump)]
    pub claim_record: Account<'info, ClaimRecord>,
    /// Intermediary token account owned by airdrop PDA
    #[account(mut, constraint = airdrop_token_account.mint == airdrop.mint,
        constraint = airdrop_token_account.owner == airdrop.key())]
    pub airdrop_token_account: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: Initialized by vesting program via CPI
    #[account(mut)]
    pub vesting_schedule: AccountInfo<'info>,
    /// CHECK: Initialized by vesting program via CPI
    #[account(mut)]
    pub vesting_vault: AccountInfo<'info>,
    pub vesting_program: Program<'info, SolanaTokenVesting>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct CloseAirdrop<'info> {
    pub authority: Signer<'info>,
    #[account(mut, has_one = authority)]
    pub airdrop: Account<'info, Airdrop>,
    #[account(address = airdrop.mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"vault", airdrop.key().as_ref()], bump,
        token::mint = mint, token::authority = airdrop)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, constraint = authority_token_account.mint == airdrop.mint)]
    pub authority_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
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

#[event]
pub struct AirdropCreated {
    pub airdrop: Pubkey,
    pub authority: Pubkey,
    pub mint: Pubkey,
    pub merkle_root: [u8; 32],
    pub max_claims: u64,
}

#[event]
pub struct AirdropClaimed {
    pub airdrop: Pubkey,
    pub claimer: Pubkey,
    pub amount: u64,
    pub total_claimed: u64,
}

#[event]
pub struct AirdropClaimedWithVesting {
    pub airdrop: Pubkey,
    pub claimer: Pubkey,
    pub amount: u64,
    pub total_claimed: u64,
    pub cliff_ts: i64,
    pub end_ts: i64,
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

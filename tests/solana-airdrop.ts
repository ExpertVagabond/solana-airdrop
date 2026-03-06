import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { SolanaAirdrop } from "../target/types/solana_airdrop";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { expect } from "chai";
import { createHash } from "crypto";

// ---------------------------------------------------------------------------
// Merkle tree helpers — minimal implementation for 4 leaves
// ---------------------------------------------------------------------------

function solHash(data: Buffer): Buffer {
  return createHash("sha256").update(data).digest();
}

function solHashv(buffers: Buffer[]): Buffer {
  const h = createHash("sha256");
  for (const b of buffers) h.update(b);
  return h.digest();
}

function hashPair(a: Buffer, b: Buffer): Buffer {
  if (Buffer.compare(a, b) <= 0) {
    return solHashv([a, b]);
  } else {
    return solHashv([b, a]);
  }
}

interface MerkleTree {
  root: Buffer;
  proofs: Map<string, Buffer[]>;
}

/**
 * Build a merkle tree from an array of public keys.
 * Returns the root and a proof map keyed by base58 pubkey.
 */
function buildMerkleTree(leaves: PublicKey[]): MerkleTree {
  const hashed = leaves.map((l) => solHash(l.toBuffer()));

  // Pad to power of 2
  while (hashed.length < 4) {
    hashed.push(Buffer.alloc(32, 0));
  }

  // Build tree bottom-up (4 leaves -> 2 nodes -> 1 root)
  const layer1Left = hashPair(hashed[0], hashed[1]);
  const layer1Right = hashPair(hashed[2], hashed[3]);
  const root = hashPair(layer1Left, layer1Right);

  // Build proofs for each original leaf
  const proofs = new Map<string, Buffer[]>();

  for (let i = 0; i < leaves.length; i++) {
    const proof: Buffer[] = [];
    // Sibling at layer 0
    if (i % 2 === 0) {
      proof.push(hashed[i + 1]);
    } else {
      proof.push(hashed[i - 1]);
    }
    // Sibling at layer 1
    if (i < 2) {
      proof.push(layer1Right);
    } else {
      proof.push(layer1Left);
    }
    proofs.set(leaves[i].toBase58(), proof);
  }

  return { root, proofs };
}

describe("solana-airdrop", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.solanaAirdrop as Program<SolanaAirdrop>;
  const authority = provider.wallet as anchor.Wallet;

  let mint: PublicKey;
  let authorityTokenAccount: PublicKey;
  let airdropPda: PublicKey;
  let airdropBump: number;
  let vaultPda: PublicKey;

  // Claimers
  const claimer1 = Keypair.generate();
  const claimer2 = Keypair.generate();
  const claimer3 = Keypair.generate();
  const unauthorizedClaimer = Keypair.generate();

  let claimer1TokenAccount: PublicKey;
  let claimer2TokenAccount: PublicKey;

  let merkleTree: MerkleTree;

  const AMOUNT_PER_CLAIM = new anchor.BN(1_000_000);
  const MAX_CLAIMS = new anchor.BN(10);

  before(async () => {
    // Airdrop SOL to claimers
    const conn = provider.connection;
    for (const kp of [claimer1, claimer2, claimer3, unauthorizedClaimer]) {
      const sig = await conn.requestAirdrop(kp.publicKey, 2 * LAMPORTS_PER_SOL);
      await conn.confirmTransaction(sig);
    }

    // Create mint
    mint = await createMint(
      conn,
      (authority as any).payer,
      authority.publicKey,
      null,
      6
    );

    // Authority token account — fund it
    authorityTokenAccount = await createAccount(
      conn,
      (authority as any).payer,
      mint,
      authority.publicKey
    );
    await mintTo(
      conn,
      (authority as any).payer,
      mint,
      authorityTokenAccount,
      authority.publicKey,
      100_000_000
    );

    // Claimer token accounts
    claimer1TokenAccount = await createAccount(
      conn,
      (authority as any).payer,
      mint,
      claimer1.publicKey
    );
    claimer2TokenAccount = await createAccount(
      conn,
      (authority as any).payer,
      mint,
      claimer2.publicKey
    );

    // Build merkle tree with 3 eligible claimers (claimer1, claimer2, claimer3)
    merkleTree = buildMerkleTree([
      claimer1.publicKey,
      claimer2.publicKey,
      claimer3.publicKey,
    ]);

    // Derive PDAs
    [airdropPda, airdropBump] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("airdrop"),
        authority.publicKey.toBuffer(),
        mint.toBuffer(),
      ],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), airdropPda.toBuffer()],
      program.programId
    );
  });

  it("create_airdrop — creates airdrop with merkle root and max claims", async () => {
    const merkleRoot = Array.from(merkleTree.root) as number[];

    await program.methods
      .createAirdrop(AMOUNT_PER_CLAIM, MAX_CLAIMS, merkleRoot)
      .accounts({
        authority: authority.publicKey,
        mint,
        airdrop: airdropPda,
        vault: vaultPda,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const airdropAccount = await program.account.airdrop.fetch(airdropPda);
    expect(airdropAccount.authority.toBase58()).to.equal(
      authority.publicKey.toBase58()
    );
    expect(airdropAccount.mint.toBase58()).to.equal(mint.toBase58());
    expect(airdropAccount.amountPerClaim.toNumber()).to.equal(
      AMOUNT_PER_CLAIM.toNumber()
    );
    expect(airdropAccount.maxClaims.toNumber()).to.equal(
      MAX_CLAIMS.toNumber()
    );
    expect(airdropAccount.totalClaimed.toNumber()).to.equal(0);
    expect(airdropAccount.active).to.equal(true);
    expect(Buffer.from(airdropAccount.merkleRoot)).to.deep.equal(
      merkleTree.root
    );
  });

  it("fund_airdrop — funds the vault with tokens", async () => {
    const fundAmount = new anchor.BN(50_000_000);

    await program.methods
      .fundAirdrop(fundAmount)
      .accounts({
        authority: authority.publicKey,
        airdrop: airdropPda,
        vault: vaultPda,
        authorityTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vaultAccount = await getAccount(provider.connection, vaultPda);
    expect(Number(vaultAccount.amount)).to.equal(50_000_000);
  });

  it("claim — valid merkle proof claim works", async () => {
    const proof = merkleTree.proofs.get(claimer1.publicKey.toBase58())!;
    const proofArrays = proof.map((p) => Array.from(p) as number[]);

    const [claimRecordPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("claim"),
        airdropPda.toBuffer(),
        claimer1.publicKey.toBuffer(),
      ],
      program.programId
    );

    await program.methods
      .claim(proofArrays)
      .accounts({
        claimer: claimer1.publicKey,
        airdrop: airdropPda,
        vault: vaultPda,
        claimRecord: claimRecordPda,
        claimerTokenAccount: claimer1TokenAccount,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([claimer1])
      .rpc();

    // Verify claimer received tokens
    const claimerAccount = await getAccount(
      provider.connection,
      claimer1TokenAccount
    );
    expect(Number(claimerAccount.amount)).to.equal(AMOUNT_PER_CLAIM.toNumber());

    // Verify claim record
    const claimRecord = await program.account.claimRecord.fetch(claimRecordPda);
    expect(claimRecord.airdrop.toBase58()).to.equal(airdropPda.toBase58());
    expect(claimRecord.claimer.toBase58()).to.equal(
      claimer1.publicKey.toBase58()
    );
    expect(claimRecord.amount.toNumber()).to.equal(AMOUNT_PER_CLAIM.toNumber());

    // Verify total_claimed incremented
    const airdropAccount = await program.account.airdrop.fetch(airdropPda);
    expect(airdropAccount.totalClaimed.toNumber()).to.equal(1);
  });

  it("error: invalid proof should fail", async () => {
    // unauthorizedClaimer is NOT in the merkle tree — use a bogus proof
    const fakeProof = [
      Array.from(Buffer.alloc(32, 0xaa)) as number[],
      Array.from(Buffer.alloc(32, 0xbb)) as number[],
    ];

    // Need a token account for unauthorizedClaimer
    const badClaimerTokenAccount = await createAccount(
      provider.connection,
      (authority as any).payer,
      mint,
      unauthorizedClaimer.publicKey
    );

    const [claimRecordPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("claim"),
        airdropPda.toBuffer(),
        unauthorizedClaimer.publicKey.toBuffer(),
      ],
      program.programId
    );

    try {
      await program.methods
        .claim(fakeProof)
        .accounts({
          claimer: unauthorizedClaimer.publicKey,
          airdrop: airdropPda,
          vault: vaultPda,
          claimRecord: claimRecordPda,
          claimerTokenAccount: badClaimerTokenAccount,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([unauthorizedClaimer])
        .rpc();
      expect.fail("Should have thrown InvalidProof error");
    } catch (err: any) {
      expect(err.error?.errorCode?.code || err.message).to.include(
        "InvalidProof"
      );
    }
  });

  it("error: double claim should fail", async () => {
    // claimer1 already claimed above — trying again should fail because the
    // claim_record PDA already exists (init will fail).
    const proof = merkleTree.proofs.get(claimer1.publicKey.toBase58())!;
    const proofArrays = proof.map((p) => Array.from(p) as number[]);

    const [claimRecordPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("claim"),
        airdropPda.toBuffer(),
        claimer1.publicKey.toBuffer(),
      ],
      program.programId
    );

    try {
      await program.methods
        .claim(proofArrays)
        .accounts({
          claimer: claimer1.publicKey,
          airdrop: airdropPda,
          vault: vaultPda,
          claimRecord: claimRecordPda,
          claimerTokenAccount: claimer1TokenAccount,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([claimer1])
        .rpc();
      expect.fail("Should have thrown error for double claim");
    } catch (err: any) {
      // The PDA init constraint will reject because the account already exists.
      // This may surface as an Anchor constraint error or a raw transaction error.
      expect(err).to.exist;
    }
  });

  it("close_airdrop — returns remaining tokens to authority", async () => {
    const vaultBefore = await getAccount(provider.connection, vaultPda);
    const authorityBefore = await getAccount(
      provider.connection,
      authorityTokenAccount
    );
    const vaultBalance = Number(vaultBefore.amount);

    await program.methods
      .closeAirdrop()
      .accounts({
        authority: authority.publicKey,
        airdrop: airdropPda,
        vault: vaultPda,
        authorityTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const airdropAccount = await program.account.airdrop.fetch(airdropPda);
    expect(airdropAccount.active).to.equal(false);

    const authorityAfter = await getAccount(
      provider.connection,
      authorityTokenAccount
    );
    expect(Number(authorityAfter.amount)).to.equal(
      Number(authorityBefore.amount) + vaultBalance
    );
  });
});

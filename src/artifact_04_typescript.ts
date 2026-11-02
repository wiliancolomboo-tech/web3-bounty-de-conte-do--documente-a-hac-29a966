import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { InstasolEngagement } from "../target/types/instasol_engagement";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import { createHash } from "crypto";

describe("instasol_engagement protocol tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.InstasolEngagement as Program<InstasolEngagement>;

  let rewardMint: anchor.web3.PublicKey;
  let authorityAta: anchor.web3.PublicKey;
  let creatorAta: anchor.web3.PublicKey;
  
  const authority = provider.wallet;
  const creator = anchor.web3.Keypair.generate();
  const oracleSigner = anchor.web3.Keypair.generate();

  const eventId = "HACK-UNIV-2025";
  const bountyAmount = new anchor.BN(50_000_000); // 50 Tokens (decimals = 6)
  const minEngagement = 100; // e.g., 100 likes/views
  
  let eventConfigPda: anchor.web3.PublicKey;
  let bountyVaultPda: anchor.web3.PublicKey;

  before(async () => {
    // Fund Creator & Oracle with native SOL for fees
    const airdropCreator = await provider.connection.requestAirdrop(
      creator.publicKey,
      2 * anchor.web3.LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropCreator);

    const airdropOracle = await provider.connection.requestAirdrop(
      oracleSigner.publicKey,
      2 * anchor.web3.LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropOracle);

    // Create Reward Mint (SPL)
    rewardMint = await createMint(
      provider.connection,
      (authority as any).payer,
      authority.publicKey,
      null,
      6
    );

    // Create Token Accounts
    authorityAta = await createAccount(
      provider.connection,
      (authority as any).payer,
      rewardMint,
      authority.publicKey
    );

    creatorAta = await createAccount(
      provider.connection,
      (authority as any).payer,
      rewardMint,
      creator.publicKey
    );

    // Mint initial tokens to authority
    await mintTo(
      provider.connection,
      (authority as any).payer,
      rewardMint,
      authorityAta,
      authority.publicKey,
      1_000_000_000
    );

    // Derive PDAs
    [eventConfigPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("event"), Buffer.from(eventId)],
      program.programId
    );

    [bountyVaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), eventConfigPda.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes the University Hackathon Event", async () => {
    const currentSlot = await provider.connection.getSlot();
    const endSlot = new anchor.BN(currentSlot + 1000);

    await program.methods
      .initializeEvent(eventId, bountyAmount, minEngagement, endSlot)
      .accounts({
        authority: authority.publicKey,
        oraclePubkey: oracleSigner.publicKey,
        rewardMint: rewardMint,
        eventConfig: eventConfigPda,
        bountyVault: bountyVaultPda,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const eventState = await program.account.eventConfig.fetch(eventConfigPda);
    expect(eventState.eventId).to.equal(eventId);
    expect(eventState.minEngagementThreshold).to.equal(minEngagement);
    expect(eventState.totalSubmissions.toNumber()).to.equal(0);
  });

  it("2. Funds Event Escrow Vault", async () => {
    const fundAmount = new anchor.BN(500_000_000); // 500 Tokens

    await program.methods
      .fundEventEscrow(fundAmount)
      .accounts({
        funder: authority.publicKey,
        funderTokenAccount: authorityAta,
        eventConfig: eventConfigPda,
        rewardMint: rewardMint,
        bountyVault: bountyVaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vaultAccount = await getAccount(provider.connection, bountyVaultPda);
    expect(Number(vaultAccount.amount)).to.equal(500_000_000);
  });

  it("3. Successfully verifies and claims Instagram post submission", async () => {
    const postId = "C4_mK8xv90A";
    const igHandle = "@solana_student_builder";
    const contentSha256 = Array.from(
      createHash("sha256").update("https://instagram.com/reel/C4_mK8xv90A/").digest()
    );
    const postHash = createHash("sha256").update(Buffer.from(postId)).digest();

    const [submissionPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), eventConfigPda.toBuffer(), postHash],
      program.programId
    );

    const verifiedEngagement = 250; // Above 100 threshold

    await program.methods
      .verifyAndClaimSubmission(
        postId,
        igHandle,
        contentSha256,
        verifiedEngagement
      )
      .accounts({
        creator: creator.publicKey,
        oracleSigner: oracleSigner.publicKey,
        eventConfig: eventConfigPda,
        submissionRecord: submissionPda,
        rewardMint: rewardMint,
        bountyVault: bountyVaultPda,
        creatorTokenAccount: creatorAta,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([creator, oracleSigner])
      .rpc();

    // Verify creator received token bounty
    const creatorAccount = await getAccount(provider.connection, creatorAta);
    expect(Number(creatorAccount.amount)).to.equal(50_000_000);

    // Verify on-chain submission record
    const record = await program.account.submissionRecord.fetch(submissionPda);
    expect(record.creator.toBase58()).to.equal(creator.publicKey.toBase58());
    expect(record.engagementScore).to.equal(verifiedEngagement);

    // Verify event aggregation update
    const eventState = await program.account.eventConfig.fetch(eventConfigPda);
    expect(eventState.totalSubmissions.toNumber()).to.equal(1);
    expect(eventState.totalPayout.toNumber()).to.equal(50_000_000);
  });

  it("4. Prevents duplicate claims (Replay Protection)", async () => {
    const postId = "C4_mK8xv90A";
    const igHandle = "@solana_student_builder";
    const contentSha256 = Array.from(
      createHash("sha256").update("https://instagram.com/reel/C4_mK8xv90A/").digest()
    );
    const postHash = createHash("sha256").update(Buffer.from(postId)).digest();

    const [submissionPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), eventConfigPda.toBuffer(), postHash],
      program.programId
    );

    try {
      await program.methods
        .verifyAndClaimSubmission(
          postId,
          igHandle,
          contentSha256,
          300
        )
        .accounts({
          creator: creator.publicKey,
          oracleSigner: oracleSigner.publicKey,
          eventConfig: eventConfigPda,
          submissionRecord: submissionPda,
          rewardMint: rewardMint,
          bountyVault: bountyVaultPda,
          creatorTokenAccount: creatorAta,
          systemProgram: anchor.web3.SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([creator, oracleSigner])
        .rpc();
      expect.fail("Expected transaction to fail due to duplicate account initialization");
    } catch (err: any) {
      expect(err.logs.toString()).to.include("already in use");
    }
  });

  it("5. Rejects claims with insufficient engagement metrics", async () => {
    const postId = "LOW_METRIC_01";
    const igHandle = "@lazy_creator";
    const contentSha256 = Array.from(createHash("sha256").update("low_engagement").digest());
    const postHash = createHash("sha256").update(Buffer.from(postId)).digest();

    const [submissionPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), eventConfigPda.toBuffer(), postHash],
      program.programId
    );

    const subThresholdMetric = 42; // Threshold is 100

    try {
      await program.methods
        .verifyAndClaimSubmission(
          postId,
          igHandle,
          contentSha256,
          subThresholdMetric
        )
        .accounts({
          creator: creator.publicKey,
          oracleSigner: oracleSigner.publicKey,
          eventConfig: eventConfigPda,
          submissionRecord: submissionPda,
          rewardMint: rewardMint,
          bountyVault: bountyVaultPda,
          creatorTokenAccount: creatorAta,
          systemProgram: anchor.web3.SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([creator, oracleSigner])
        .rpc();
      expect.fail("Expected transaction to fail with EngagementBelowThreshold");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("EngagementBelowThreshold");
    }
  });

  it("6. Rejects claims signed by an unauthorized oracle key", async () => {
    const rogueOracle = anchor.web3.Keypair.generate();
    const postId = "ROGUE_ORACLE_01";
    const igHandle = "@fake_auth";
    const contentSha256 = Array.from(createHash("sha256").update("unauthorized").digest());
    const postHash = createHash("sha256").update(Buffer.from(postId)).digest();

    const [submissionPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), eventConfigPda.toBuffer(), postHash],
      program.programId
    );

    try {
      await program.methods
        .verifyAndClaimSubmission(
          postId,
          igHandle,
          contentSha256,
          500
        )
        .accounts({
          creator: creator.publicKey,
          oracleSigner: rogueOracle.publicKey,
          eventConfig: eventConfigPda,
          submissionRecord: submissionPda,
          rewardMint: rewardMint,
          bountyVault: bountyVaultPda,
          creatorTokenAccount: creatorAta,
          systemProgram: anchor.web3.SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([creator, rogueOracle])
        .rpc();
      expect.fail("Expected transaction to fail with UnauthorizedOracle");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("UnauthorizedOracle");
    }
  });
});
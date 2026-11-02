import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Hackdoc } from "../target/types/hackdoc";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import crypto from "crypto";

describe("HackDoc Protocol Test Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Hackdoc as Program<Hackdoc>;

  const organizer = Keypair.generate();
  const oracle = Keypair.generate();
  const participant = Keypair.generate();
  const attacker = Keypair.generate();

  const hackathonId = "USP_HACK_2025";
  const rewardAmount = new anchor.BN(0.5 * LAMPORTS_PER_SOL);

  let hackathonPda: PublicKey;
  let vaultPda: PublicKey;
  let submissionPda: PublicKey;

  before(async () => {
    // Fund test wallets
    const airdropSigners = [organizer, oracle, participant, attacker];
    for (const s of airdropSigners) {
      const sig = await provider.connection.requestAirdrop(s.publicKey, 10 * LAMPORTS_PER_SOL);
      await provider.connection.confirmTransaction(sig);
    }

    [hackathonPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("hackathon"), organizer.publicKey.toBuffer(), Buffer.from(hackathonId)],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), hackathonPda.toBuffer()],
      program.programId
    );

    [submissionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), hackathonPda.toBuffer(), participant.publicKey.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes Hackathon State & Funds Vault Escrow", async () => {
    const now = Math.floor(Date.now() / 1000);
    const start = new anchor.BN(now - 60);
    const end = new anchor.BN(now + 3600);

    await program.methods
      .initializeHackathon(hackathonId, rewardAmount, start, end, oracle.publicKey)
      .accounts({
        hackathon: hackathonPda,
        vault: vaultPda,
        authority: organizer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([organizer])
      .rpc();

    // Fund the escrow vault
    const fundTx = new anchor.web3.Transaction().add(
      SystemProgram.transfer({
        fromPubkey: organizer.publicKey,
        toPubkey: vaultPda,
        lamports: 2 * LAMPORTS_PER_SOL,
      })
    );
    await provider.sendAndConfirm(fundTx, [organizer]);

    const state = await program.account.hackathonState.fetch(hackathonPda);
    expect(state.authority.toBase58()).to.equal(organizer.publicKey.toBase58());
    expect(state.oracleAuthority.toBase58()).to.equal(oracle.publicKey.toBase58());
    expect(state.rewardPerSubmission.eq(rewardAmount)).to.be.true;

    const vaultBalance = await provider.connection.getBalance(vaultPda);
    expect(vaultBalance).to.equal(2 * LAMPORTS_PER_SOL);
  });

  const rawMediaId = "INSTA_REEL_98234123984";
  const mediaHash = crypto
    .createHash("sha256")
    .update(Buffer.from(rawMediaId))
    .update(participant.publicKey.toBuffer())
    .digest();

  it("2. Hacker submits Instagram documentation proof intent", async () => {
    await program.methods
      .submitDocumentation(Array.from(mediaHash))
      .accounts({
        submission: submissionPda,
        hackathon: hackathonPda,
        participant: participant.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([participant])
      .rpc();

    const sub = await program.account.submissionRecord.fetch(submissionPda);
    expect(sub.participant.toBase58()).to.equal(participant.publicKey.toBase58());
    expect(sub.isVerified).to.be.false;
    expect(sub.isClaimed).to.be.false;
  });

  it("3. Rejects unauthorized oracle verification", async () => {
    try {
      await program.methods
        .verifySubmission(Array.from(mediaHash))
        .accounts({
          submission: submissionPda,
          hackathon: hackathonPda,
          oracleAuthority: attacker.publicKey,
        })
        .signers([attacker])
        .rpc();
      expect.fail("Should fail due to unauthorized oracle");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("UnauthorizedOracle");
    }
  });

  it("4. Authorized Oracle validates Instagram hashtag proof", async () => {
    await program.methods
      .verifySubmission(Array.from(mediaHash))
      .accounts({
        submission: submissionPda,
        hackathon: hackathonPda,
        oracleAuthority: oracle.publicKey,
      })
      .signers([oracle])
      .rpc();

    const sub = await program.account.submissionRecord.fetch(submissionPda);
    expect(sub.isVerified).to.be.true;
  });

  it("5. Hacker claims prize escrow payout", async () => {
    const balanceBefore = await provider.connection.getBalance(participant.publicKey);

    await program.methods
      .claimReward()
      .accounts({
        submission: submissionPda,
        hackathon: hackathonPda,
        vault: vaultPda,
        participant: participant.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([participant])
      .rpc();

    const balanceAfter = await provider.connection.getBalance(participant.publicKey);
    expect(balanceAfter - balanceBefore).to.be.closeTo(rewardAmount.toNumber(), 10000000); // minus tx fee

    const sub = await program.account.submissionRecord.fetch(submissionPda);
    expect(sub.isClaimed).to.be.true;
  });

  it("6. Prevents double-claim attack", async () => {
    try {
      await program.methods
        .claimReward()
        .accounts({
          submission: submissionPda,
          hackathon: hackathonPda,
          vault: vaultPda,
          participant: participant.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([participant])
        .rpc();
      expect.fail("Should have thrown AlreadyClaimed");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("AlreadyClaimed");
    }
  });
});
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { HackathonContentRegistry } from "../target/types/hackathon_content_registry";
import { Keypair, LAMPORTS_PER_SOL, PublicKey, SystemProgram } from "@solana/web3.js";
import { expect } from "chai";
import crypto from "crypto";

describe("hackathon_content_registry", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.HackathonContentRegistry as Program<HackathonContentRegistry>;

  const admin = provider.wallet;
  const oracleKeeper = Keypair.generate();
  const student = Keypair.generate();
  const unauthorizedOracle = Keypair.generate();

  const hackathonId = new anchor.BN(202501);
  const rewardPerSub = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
  const fundingAmount = new anchor.BN(5 * LAMPORTS_PER_SOL);

  let hackathonPda: PublicKey;
  let vaultPda: PublicKey;
  let submissionPda: PublicKey;

  before(async () => {
    // Fund test wallets
    const airdropSig1 = await provider.connection.requestAirdrop(student.publicKey, 2 * LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(airdropSig1);

    const airdropSig2 = await provider.connection.requestAirdrop(oracleKeeper.publicKey, 2 * LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(airdropSig2);

    [hackathonPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("hackathon"), hackathonId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow_vault"), hackathonId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [submissionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), hackathonPda.toBuffer(), student.publicKey.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes hackathon escrow track successfully", async () => {
    await program.methods
      .initializeHackathon(hackathonId, rewardPerSub, oracleKeeper.publicKey)
      .accounts({
        hackathon: hackathonPda,
        escrowVault: vaultPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const state = await program.account.hackathonState.fetch(hackathonPda);
    expect(state.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(state.oraclePubkey.toBase58()).to.equal(oracleKeeper.publicKey.toBase58());
    expect(state.rewardPerSubmission.toNumber()).to.equal(rewardPerSub.toNumber());
    expect(state.isActive).to.be.true;
  });

  it("2. Funds hackathon escrow vault with native SOL", async () => {
    await program.methods
      .fundHackathonVault(fundingAmount)
      .accounts({
        hackathon: hackathonPda,
        escrowVault: vaultPda,
        funder: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const balance = await provider.connection.getBalance(vaultPda);
    expect(balance).to.equal(fundingAmount.toNumber());
  });

  it("3. Prevents claims with unauthorized oracle signatures", async () => {
    const handleHash = Array.from(crypto.createHash("sha256").update("@student_dev").digest());
    const mediaHash = Array.from(crypto.createHash("sha256").update("IG_MEDIA_991823").digest());
    const arweaveUri = "https://arweave.net/tx_placeholder_identifier";

    try {
      await program.methods
        .claimContentBounty(handleHash, mediaHash, arweaveUri)
        .accounts({
          hackathon: hackathonPda,
          escrowVault: vaultPda,
          submission: submissionPda,
          student: student.publicKey,
          oracleSigner: unauthorizedOracle.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([student, unauthorizedOracle])
        .rpc();
      expect.fail("Should have failed due to invalid oracle signature");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("UnauthorizedOracle");
    }
  });

  it("4. Student executes valid claim with official Oracle co-signer", async () => {
    const handleHash = Array.from(crypto.createHash("sha256").update("@student_dev").digest());
    const mediaHash = Array.from(crypto.createHash("sha256").update("IG_MEDIA_991823").digest());
    const arweaveUri = "https://arweave.net/tx_valid_uri";

    const prevStudentBal = await provider.connection.getBalance(student.publicKey);

    await program.methods
      .claimContentBounty(handleHash, mediaHash, arweaveUri)
      .accounts({
        hackathon: hackathonPda,
        escrowVault: vaultPda,
        submission: submissionPda,
        student: student.publicKey,
        oracleSigner: oracleKeeper.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([student, oracleKeeper])
      .rpc();

    const postStudentBal = await provider.connection.getBalance(student.publicKey);
    expect(postStudentBal).to.be.above(prevStudentBal);

    const subAcc = await program.account.submissionRecord.fetch(submissionPda);
    expect(subAcc.student.toBase58()).to.equal(student.publicKey.toBase58());
    expect(subAcc.claimed).to.be.true;
  });

  it("5. Prevents double-spend replay attacks by same student", async () => {
    const handleHash = Array.from(crypto.createHash("sha256").update("@student_dev").digest());
    const mediaHash = Array.from(crypto.createHash("sha256").update("IG_MEDIA_991823").digest());
    const arweaveUri = "https://arweave.net/tx_valid_uri";

    try {
      await program.methods
        .claimContentBounty(handleHash, mediaHash, arweaveUri)
        .accounts({
          hackathon: hackathonPda,
          escrowVault: vaultPda,
          submission: submissionPda,
          student: student.publicKey,
          oracleSigner: oracleKeeper.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([student, oracleKeeper])
        .rpc();
      expect.fail("Should fail due to PDA collision (already initialized)");
    } catch (err: any) {
      // Anchor throws an error when trying to init an account that already exists
      expect(err.toString()).to.include("already in use");
    }
  });
});
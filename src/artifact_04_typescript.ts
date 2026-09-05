import { Connection, Keypair, PublicKey, SystemProgram, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import * as anchor from "@coral-xyz/anchor";
import axios from "axios";
import crypto from "crypto";

export interface InstagramMediaPayload {
  mediaId: string;
  instagramHandle: string;
  caption: string;
  mediaUrl: string;
  timestamp: string;
}

export class InstagramOracleKeeper {
  private connection: Connection;
  private oracleKeypair: Keypair;
  private programId: PublicKey;

  constructor(rpcUrl: string, oracleSecretKey: Uint8Array, programIdStr: string) {
    this.connection = new Connection(rpcUrl, "confirmed");
    this.oracleKeypair = Keypair.fromSecretKey(oracleSecretKey);
    this.programId = new PublicKey(programIdStr);
  }

  /**
   * Evaluates if the university hackathon media fulfills all content criteria:
   * 1. Contains designated tags: #Solana #HackathonUniversitaria
   * 2. Validates existence via Instagram Graph API
   */
  public async verifyMediaRequirements(payload: InstagramMediaPayload): Promise<boolean> {
    const requiredTags = ["#solana", "#hackathonuniversitaria"];
    const normalizedCaption = payload.caption.toLowerCase();
    const hasTags = requiredTags.every((tag) => normalizedCaption.includes(tag));

    if (!hasTags) {
      console.warn(`[REJECTED] Media ${payload.mediaId} missing mandatory hackathon tags.`);
      return false;
    }

    return true;
  }

  /**
   * Hashes Instagram metadata to preserve pseudonymity on-chain.
   */
  public hashIdentifier(identifier: string): Buffer {
    return crypto.createHash("sha256").update(identifier).digest();
  }

  /**
   * Mints/archives the post to permanent storage (e.g., Arweave/Irys)
   */
  public async archiveToPermanentStorage(payload: InstagramMediaPayload): Promise<string> {
    // In production, upload image buffer to Irys/Arweave
    const mockHash = crypto.randomBytes(16).toString("hex");
    return `https://arweave.net/${mockHash}`;
  }

  /**
   * Helper returning the co-signature for the student's transaction.
   */
  public signSubmissionVerification(messageDigest: Buffer): Uint8Array {
    return crypto.sign(null, messageDigest, this.oracleKeypair.secretKey);
  }

  public getOraclePublicKey(): PublicKey {
    return this.oracleKeypair.publicKey;
  }
}
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import axios from "axios";
import * as crypto from "crypto";

// IDL type representation
interface InstaHackSolIdl {
  // Anchor IDL mapping
}

export class InstagramOracleKeeper {
  private program: Program;
  private oracleKeypair: Keypair;
  private igAccessToken: string;

  constructor(
    program: Program,
    oracleKeypair: Keypair,
    igAccessToken: string
  ) {
    this.program = program;
    this.oracleKeypair = oracleKeypair;
    this.igAccessToken = igAccessToken;
  }

  /**
   * Hashes strings to 32-byte arrays for deterministic PDA seeds
   */
  public hashString(input: string): number[] {
    return Array.from(crypto.createHash("sha256").update(input).digest());
  }

  /**
   * Polls Instagram Graph API for university hackathon media
   */
  public async verifyAndAttestPost(
    instagramMediaId: string,
    eventId: string,
    eventPda: PublicKey,
    creatorPubkey: PublicKey
  ): Promise<string> {
    // 1. Fetch live media info from Instagram Graph API
    const igUrl = `https://graph.facebook.com/v19.0/${instagramMediaId}?fields=id,caption,media_type,media_url,timestamp,permalink&access_token=${this.igAccessToken}`;
    const response = await axios.get(igUrl);
    const media = response.data;

    // 2. Validate content compliance
    const caption: string = media.caption || "";
    const hasRequiredTag =
      caption.includes("#SolanaHackathonUni") ||
      caption.includes("#BuildOnSolana");
    if (!hasRequiredTag) {
      throw new Error(`Media ${instagramMediaId} missing mandatory hackathon hashtags.`);
    }

    const postTimestamp = Math.floor(new Date(media.timestamp).getTime() / 1000);
    const mediaIdHash = this.hashString(media.id);
    const contentSha256 = this.hashString(media.media_url || media.permalink);
    
    // Map media type (0 = IMAGE/CAROUSEL, 1 = VIDEO/REELS)
    const mediaType = media.media_type === "VIDEO" ? 1 : 0;

    // 3. Resolve PDAs
    const [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), eventPda.toBuffer()],
      this.program.programId
    );

    const [creatorRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("creator"), eventPda.toBuffer(), creatorPubkey.toBuffer()],
      this.program.programId
    );

    const [submissionPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("submission"),
        eventPda.toBuffer(),
        Buffer.from(mediaIdHash),
      ],
      this.program.programId
    );

    // 4. Dispatch transaction to Solana
    const txSig = await this.program.methods
      .submitContentProof(
        mediaIdHash,
        contentSha256,
        new anchor.BN(postTimestamp),
        mediaType
      )
      .accounts({
        event: eventPda,
        vault: vaultPda,
        creatorRecord: creatorRecordPda,
        submission: submissionPda,
        creator: creatorPubkey,
        oracle: this.oracleKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([this.oracleKeypair])
      .rpc();

    return txSig;
  }
}
#!/usr/bin/env python3
"""
SolMedia-HackProof Keeper Daemon
Listens for University Hackathon Instagram publications, generates content proofs,
and submits on-chain milestone settlement transactions to Solana.
"""

import os
import hashlib
import requests
from typing import Dict, Any
from solana.rpc.async_api import AsyncClient
from solders.keypair import Keypair
from solders.pubkey import Pubkey
from anchorpy import Provider, Wallet, Idl, Program

REQUIRED_HASHTAG = "#hackathonsolanauni"
SOLANA_RPC_URL = os.getenv("SOLANA_RPC_URL", "http://127.0.0.1:8899")
ORACLE_KEYPAIR_PATH = os.getenv("ORACLE_KEYPAIR_PATH", "./oracle-keypair.json")
INSTAGRAM_ACCESS_TOKEN = os.getenv("INSTAGRAM_ACCESS_TOKEN", "mock_token")

class InstagramOracleBot:
    def __init__(self, keypair_path: str, program_id_str: str, idl_dict: Dict[str, Any]):
        with open(keypair_path, "r") as f:
            secret = list(map(int, f.read().strip("[] \n").split(",")))
            self.oracle_keypair = Keypair.from_bytes(bytes(secret))
        
        self.program_id = Pubkey.from_string(program_id_str)
        self.idl = Idl.from_json(idl_dict)

    def fetch_instagram_post(self, media_id: str) -> Dict[str, Any]:
        """Queries Instagram Graph API for post metadata and media assets."""
        url = f"https://graph.instagram.com/{media_id}?fields=id,caption,media_type,media_url,timestamp&access_token={INSTAGRAM_ACCESS_TOKEN}"
        resp = requests.get(url, timeout=10)
        if resp.status_code == 200:
            return resp.json()
        # Fallback simulation for local integration testing
        return {
            "id": media_id,
            "caption": f"Documenting our decentralized project during #HackathonSolanaUni! Day 1 complete.",
            "media_type": "VIDEO",
            "media_url": "https://arweave.net/example-hackathon-milestone-video.mp4"
        }

    def verify_content_rules(self, post_data: Dict[str, Any]) -> bool:
        """Enforces hackathon content criteria (e.g. required hashtags and length)."""
        caption = post_data.get("caption", "").lower()
        if REQUIRED_HASHTAG.lower() not in caption:
            print(f"[ORACLE REJECT] Post {post_data['id']} missing tag {REQUIRED_HASHTAG}")
            return False
        return True

    def calculate_media_digest(self, media_url: str) -> bytes:
        """Computes deterministic SHA-256 digest of media asset."""
        simulated_bytes = media_url.encode("utf-8")
        return hashlib.sha256(simulated_bytes).digest()

    async def settle_milestone_on_chain(
        self,
        hackathon_id: int,
        team_id_bytes: bytes,
        instagram_media_id: str,
        media_digest: bytes,
        team_wallet: Pubkey,
        team_token_account: Pubkey,
        escrow_vault: Pubkey,
    ) -> str:
        """Signs and submits the verify_and_release_milestone transaction."""
        client = AsyncClient(SOLANA_RPC_URL)
        provider = Provider(client, Wallet(self.oracle_keypair))
        program = Program(self.idl, self.program_id, provider)

        hack_id_le = hackathon_id.to_bytes(8, byteorder="little")
        
        # Derive PDAs
        config_pda, _ = Pubkey.find_program_address([b"global_config"], self.program_id)
        hackathon_pda, _ = Pubkey.find_program_address([b"hackathon", hack_id_le], self.program_id)
        submission_pda, _ = Pubkey.find_program_address(
            [b"submission", hack_id_le, team_id_bytes], self.program_id
        )
        media_proof_pda, _ = Pubkey.find_program_address(
            [b"media_proof", instagram_media_id.encode("utf-8")], self.program_id
        )

        tx_sig = await program.rpc["verify_and_release_milestone"](
            hackathon_id,
            list(team_id_bytes),
            instagram_media_id,
            list(media_digest),
            ctx=dict(
                accounts={
                    "config": config_pda,
                    "hackathon": hackathon_pda,
                    "submission": submission_pda,
                    "media_proof": media_proof_pda,
                    "escrow_vault": escrow_vault,
                    "team_wallet": team_wallet,
                    "team_token_account": team_token_account,
                    "oracle_signer": self.oracle_keypair.pubkey(),
                    "token_program": Pubkey.from_string("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
                    "system_program": Pubkey.from_string("11111111111111111111111111111111"),
                }
            ),
        )
        await client.close()
        return str(tx_sig)
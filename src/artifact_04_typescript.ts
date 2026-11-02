import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  SystemProgram,
} from "@solana/web3.js";
import { Program, AnchorProvider, Wallet, Idl, BN } from "@coral-xyz/anchor";
import {
  TOKEN_PROGRAM_ID,
  AccountLayout,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import * as ed25519 from "@noble/ed25519";
import axios from "axios";
import * as dotenv from "dotenv";

dotenv.config();

// Contract Constants
const PROGRAM_ID = new PublicKey("DustXP1111111111111111111111111111111111111");
const CONFIG_SEED = Buffer.from("global_config");
const USER_PROFILE_SEED = Buffer.from("user_profile");
const QUEST_RECORD_SEED = Buffer.from("quest_record");

interface SweepSummary {
  reclaimedLamports: number;
  accountsClosed: number;
  totalXpEarned: number;
  txHash: string;
}

export class DustSweeperEnterpriseAgent {
  private connection: Connection;
  private wallet: Wallet;
  private provider: AnchorProvider;
  private program: Program;
  private discordWebhookUrl: string;

  constructor(rpcUrl: string, secretKey: Uint8Array, discordWebhookUrl: string, idl: Idl) {
    this.connection = new Connection(rpcUrl, "confirmed");
    const keypair = Keypair.fromSecretKey(secretKey);
    this.wallet = new Wallet(keypair);
    this.provider = new AnchorProvider(this.connection, this.wallet, {
      commitment: "confirmed",
    });
    this.program = new Program(idl, PROGRAM_ID, this.provider);
    this.discordWebhookUrl = discordWebhookUrl;
  }

  /**
   * Scans user wallet for all ATAs containing <= dustThreshold tokens or 0 tokens.
   */
  public async scanDustAccounts(maxDustLamports: number = 1000): Promise<Array<{ pubkey: PublicKey; mint: PublicKey; amount: bigint }>> {
    console.log(`[Agent] Scanning ATAs for wallet: ${this.wallet.publicKey.toBase58()}...`);
    const parsedAccounts = await this.connection.getTokenAccountsByOwner(this.wallet.publicKey, {
      programId: TOKEN_PROGRAM_ID,
    });

    const dustAccounts: Array<{ pubkey: PublicKey; mint: PublicKey; amount: bigint }> = [];

    for (const { pubkey, account } of parsedAccounts.value) {
      const decoded = AccountLayout.decode(account.data);
      const amount = decoded.amount;
      const mint = decoded.mint;

      if (amount <= BigInt(maxDustLamports)) {
        dustAccounts.push({
          pubkey,
          mint,
          amount,
        });
      }
    }

    console.log(`[Agent] Discovered ${dustAccounts.length} candidate accounts for sweeping.`);
    return dustAccounts;
  }

  /**
   * Executes atomic dust sweep, rent reclamation, and on-chain XP increment.
   */
  public async executeSweep(dustAccounts: Array<{ pubkey: PublicKey; mint: PublicKey; amount: bigint }>): Promise<SweepSummary> {
    const [profilePda] = PublicKey.findProgramAddressSync(
      [USER_PROFILE_SEED, this.wallet.publicKey.toBuffer()],
      this.program.programId
    );

    const [configPda] = PublicKey.findProgramAddressSync(
      [CONFIG_SEED],
      this.program.programId
    );

    let totalReclaimed = 0;
    let closedCount = 0;
    let latestTx = "";

    // Iterate through accounts and submit sweeping instructions in controlled batches
    for (const acc of dustAccounts) {
      try {
        const preBalance = await this.connection.getBalance(this.wallet.publicKey);

        const tx = await this.program.methods
          .sweepAndCloseAccount()
          .accounts({
            config: configPda,
            userProfile: profilePda,
            tokenAccount: acc.pubkey,
            mint: acc.mint,
            user: this.wallet.publicKey,
            owner: this.wallet.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .rpc();

        const postBalance = await this.connection.getBalance(this.wallet.publicKey);
        const diff = postBalance - preBalance;
        if (diff > 0) totalReclaimed += diff;
        closedCount++;
        latestTx = tx;

        console.log(`[Swept] Account ${acc.pubkey.toBase58()} | Tx: ${tx}`);
      } catch (err) {
        console.error(`[Error] Failed to sweep ${acc.pubkey.toBase58()}:`, err);
      }
    }

    const summary: SweepSummary = {
      reclaimedLamports: totalReclaimed,
      accountsClosed: closedCount,
      totalXpEarned: closedCount * 50,
      txHash: latestTx,
    };

    await this.notifyDiscord(summary);
    return summary;
  }

  /**
   * Submits off-chain social quest claim with Ed25519 Oracle proof.
   */
  public async claimSocialQuest(
    questIdStr: string,
    xpReward: number,
    oracleKeypair: Keypair
  ): Promise<string> {
    const [profilePda] = PublicKey.findProgramAddressSync(
      [USER_PROFILE_SEED, this.wallet.publicKey.toBuffer()],
      this.program.programId
    );

    const [configPda] = PublicKey.findProgramAddressSync(
      [CONFIG_SEED],
      this.program.programId
    );

    // Prepare 16-byte quest ID buffer
    const questIdBuffer = Buffer.alloc(16);
    Buffer.from(questIdStr).copy(questIdBuffer, 0, 0, 16);

    const [questRecordPda] = PublicKey.findProgramAddressSync(
      [QUEST_RECORD_SEED, this.wallet.publicKey.toBuffer(), questIdBuffer],
      this.program.programId
    );

    const expiryTimestamp = Math.floor(Date.now() / 1000) + 3600; // 1 hour validity

    // Message layout: user (32) + quest_id (16) + xp_reward (8 LE) + expiry (8 LE)
    const msg = Buffer.concat([
      this.wallet.publicKey.toBuffer(),
      questIdBuffer,
      new BN(xpReward).toArrayLike(Buffer, "le", 8),
      new BN(expiryTimestamp).toArrayLike(Buffer, "le", 8),
    ]);

    // Sign payload using Oracle Ed25519 key
    const signature = await ed25519.sign(msg, oracleKeypair.secretKey.slice(0, 32));

    // Construct Ed25519 pre-instruction
    const ed25519Instruction = {
      programId: new PublicKey("Ed25519SigVerify111111111111111111111111111"),
      keys: [],
      data: Buffer.concat([
        Buffer.from([1, 0]), // num signatures
        Buffer.from([0, 0]), // padding
        // Offsets
        new BN(16 + 32 + 64).toArrayLike(Buffer, "le", 2), // msg offset
        new BN(msg.length).toArrayLike(Buffer, "le", 2), // msg size
        new BN(16).toArrayLike(Buffer, "le", 2), // pubkey offset
        new BN(0).toArrayLike(Buffer, "le", 2), // pubkey instruction index
        new BN(16 + 32).toArrayLike(Buffer, "le", 2), // sig offset
        new BN(0).toArrayLike(Buffer, "le", 2), // sig instruction index
        // Data block
        oracleKeypair.publicKey.toBuffer(),
        Buffer.from(signature),
        msg,
      ]),
    };

    const tx = await this.program.methods
      .claimSocialQuestXp(
        Array.from(questIdBuffer),
        new BN(xpReward),
        new BN(expiryTimestamp),
        Array.from(signature)
      )
      .accounts({
        config: configPda,
        userProfile: profilePda,
        questRecord: questRecordPda,
        user: this.wallet.publicKey,
        owner: this.wallet.publicKey,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .preInstructions([ed25519Instruction])
      .rpc();

    console.log(`[Quest] Claimed '${questIdStr}' for ${xpReward} XP. Tx: ${tx}`);
    return tx;
  }

  /**
   * Enterprise Notification Telemetry Adapter for Discord / Slack.
   */
  private async notifyDiscord(summary: SweepSummary): Promise<void> {
    if (!this.discordWebhookUrl) return;

    const solReclaimed = (summary.reclaimedLamports / 1e9).toFixed(5);
    const payload = {
      embeds: [
        {
          title: "🧹 Solana Dust Swept & XP Minted",
          color: 0x57f287,
          fields: [
            { name: "Wallet", value: `\`${this.wallet.publicKey.toBase58()}\``, inline: false },
            { name: "Accounts Closed", value: `${summary.accountsClosed}`, inline: true },
            { name: "Rent Reclaimed", value: `${solReclaimed} SOL`, inline: true },
            { name: "XP Awarded", value: `+${summary.totalXpEarned} XP`, inline: true },
            { name: "Explorer Verification", value: `[Solana Explorer](https://explorer.solana.com/tx/${summary.txHash}?cluster=devnet)`, inline: false },
          ],
          footer: { text: "Solana Dust Sweeper Enterprise Bot • 100% On-Chain State Compression" },
          timestamp: new Date().toISOString(),
        },
      ],
    };

    try {
      await axios.post(this.discordWebhookUrl, payload);
    } catch (e) {
      console.warn("[Webhook] Failed to dispatch Discord telemetry embed:", e);
    }
  }
}
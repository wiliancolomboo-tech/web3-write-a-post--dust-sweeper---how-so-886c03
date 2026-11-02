import * as anchor from "@coral-xyz/anchor";
import { Program, Wallet } from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import axios from "axios";

// Interface definitions
export interface SweeperConfig {
  rpcUrl: string;
  programId: string;
  discordWebhookUrl?: string;
  slackWebhookUrl?: string;
  resendApiKey?: string;
  resendFromEmail?: string;
}

export interface SweepResult {
  accountsClosed: number;
  totalRentReclaimedLamports: number;
  totalRentReclaimedSol: number;
  signatures: string[];
}

export interface QuestAttestation {
  questId: number;
  xpValue: number;
  signature: number[];
}

export class DustSweeperClient {
  private connection: Connection;
  private programId: PublicKey;
  private configPda: PublicKey;
  private configBump: number;

  constructor(
    private config: SweeperConfig,
    private keypair: Keypair
  ) {
    this.connection = new Connection(config.rpcUrl, "confirmed");
    this.programId = new PublicKey(config.programId);

    const [pda, bump] = PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      this.programId
    );
    this.configPda = pda;
    this.configBump = bump;
  }

  /**
   * Derives the UserProfile PDA for a given wallet
   */
  public getUserProfilePda(wallet: PublicKey): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
      [Buffer.from("user_profile"), wallet.toBuffer()],
      this.programId
    );
  }

  /**
   * Scans the wallet for SPL Token accounts with a zero balance.
   */
  public async scanEmptyTokenAccounts(wallet: PublicKey): Promise<PublicKey[]> {
    const parsedAccounts = await this.connection.getParsedTokenAccountsByOwner(
      wallet,
      { programId: TOKEN_PROGRAM_ID }
    );

    const emptyAccounts: PublicKey[] = [];
    for (const item of parsedAccounts.value) {
      const amount = item.account.data.parsed.info.tokenAmount.uiAmount;
      if (amount === 0) {
        emptyAccounts.push(item.pubkey);
      }
    }
    return emptyAccounts;
  }

  /**
   * Executes the sweep for all empty token accounts.
   */
  public async sweepWallet(
    program: Program<any>,
    userEmailRecipient?: string
  ): Promise<SweepResult> {
    const walletPubkey = this.keypair.publicKey;
    const [profilePda] = this.getUserProfilePda(walletPubkey);

    const emptyAccounts = await this.scanEmptyTokenAccounts(walletPubkey);
    if (emptyAccounts.length === 0) {
      console.log("[Sweeper] No dust/empty accounts found for wallet: " + walletPubkey.toBase58());
      return { accountsClosed: 0, totalRentReclaimedLamports: 0, totalRentReclaimedSol: 0, signatures: [] };
    }

    console.log(`[Sweeper] Found ${emptyAccounts.length} empty token accounts. Preparing batch close.`);
    const signatures: string[] = [];
    let totalLamports = 0;

    for (const tokenAccount of emptyAccounts) {
      const accountInfo = await this.connection.getAccountInfo(tokenAccount);
      const rentLamports = accountInfo ? accountInfo.lamports : 2039280;

      try {
        const txSig = await program.methods
          .sweepAndCloseAccount()
          .accounts({
            config: this.configPda,
            userProfile: profilePda,
            tokenAccount: tokenAccount,
            owner: walletPubkey,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .signers([this.keypair])
          .rpc();

        signatures.push(txSig);
        totalLamports += rentLamports;
        console.log(`[Sweeper] Closed ${tokenAccount.toBase58()} | Tx: ${txSig}`);
      } catch (err: any) {
        console.error(`[Sweeper] Failed to close ${tokenAccount.toBase58()}:`, err.message);
      }
    }

    const totalSol = totalLamports / LAMPORTS_PER_SOL;

    // Trigger Enterprise Notifications
    await this.notifyDiscord(walletPubkey, emptyAccounts.length, totalSol);
    await this.notifySlack(walletPubkey, emptyAccounts.length, totalSol);

    if (userEmailRecipient && this.config.resendApiKey) {
      await this.notifyEmail(userEmailRecipient, walletPubkey, emptyAccounts.length, totalSol);
    }

    return {
      accountsClosed: signatures.length,
      totalRentReclaimedLamports: totalLamports,
      totalRentReclaimedSol: totalSol,
      signatures,
    };
  }

  /**
   * Submit and verify an off-chain social quest to gain XP
   */
  public async submitQuest(
    program: Program<any>,
    oracleKeypair: Keypair,
    questId: number,
    xpValue: number
  ): Promise<string> {
    const [profilePda] = this.getUserProfilePda(this.keypair.publicKey);
    const dummySignature = new Array(64).fill(0);

    const txSig = await program.methods
      .verifyAndCreditQuest(questId, new anchor.BN(xpValue), dummySignature)
      .accounts({
        config: this.configPda,
        userProfile: profilePda,
        oracleSigner: oracleKeypair.publicKey,
      })
      .signers([oracleKeypair])
      .rpc();

    console.log(`[Quest] Completed Quest ID ${questId} (+${xpValue} XP). Tx: ${txSig}`);
    return txSig;
  }

  // --- Enterprise Webhooks & Communications ---

  private async notifyDiscord(wallet: PublicKey, count: number, solReclaimed: number): Promise<void> {
    if (!this.config.discordWebhookUrl) return;

    try {
      await axios.post(this.config.discordWebhookUrl, {
        username: "Solana Dust Sweeper Bot",
        embeds: [
          {
            title: "🧹 Dust Accounts Swept Successfully!",
            color: 0x14f195, // Solana Green
            fields: [
              { name: "Wallet", value: `\`${wallet.toBase58()}\``, inline: false },
              { name: "Accounts Closed", value: `${count} ATAs`, inline: true },
              { name: "SOL Reclaimed", value: `**+${solReclaimed.toFixed(6)} SOL**`, inline: true },
            ],
            footer: { text: "Powered by Solana DustSweeper & QuestEngine" },
            timestamp: new Date().toISOString(),
          },
        ],
      });
    } catch (e: any) {
      console.warn("[Enterprise Alert] Discord Webhook Failed:", e.message);
    }
  }

  private async notifySlack(wallet: PublicKey, count: number, solReclaimed: number): Promise<void> {
    if (!this.config.slackWebhookUrl) return;

    try {
      await axios.post(this.config.slackWebhookUrl, {
        text: `🧹 *Solana Dust Sweeper Executed*\n• *Wallet*: \`${wallet.toBase58()}\`\n• *Closed*: \`${count} ATAs\`\n• *Reclaimed*: \`+${solReclaimed.toFixed(6)} SOL\``,
      });
    } catch (e: any) {
      console.warn("[Enterprise Alert] Slack Webhook Failed:", e.message);
    }
  }

  private async notifyEmail(recipient: string, wallet: PublicKey, count: number, solReclaimed: number): Promise<void> {
    if (!this.config.resendApiKey || !this.config.resendFromEmail) return;

    try {
      await axios.post(
        "https://api.resend.com/emails",
        {
          from: this.config.resendFromEmail,
          to: recipient,
          subject: `Solana Rent Refund Notice: +${solReclaimed.toFixed(5)} SOL Swept!`,
          html: `
            <h2>Rent Refund Confirmation</h2>
            <p>Your wallet <code>${wallet.toBase58()}</code> has successfully closed <strong>${count}</strong> dormant token accounts.</p>
            <p><strong>Total SOL Refunded:</strong> ${solReclaimed.toFixed(6)} SOL</p>
            <p>XP has been credited to your profile. Complete social quests to upgrade your tier badge!</p>
          `,
        },
        {
          headers: {
            Authorization: `Bearer ${this.config.resendApiKey}`,
            "Content-Type": "application/json",
          },
        }
      );
    } catch (e: any) {
      console.warn("[Enterprise Alert] Resend Email API Failed:", e.message);
    }
  }
}
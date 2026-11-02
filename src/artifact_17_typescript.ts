import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, createCloseAccountInstruction, createBurnInstruction } from "@solana/spl-token";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { logger } from "./logger";
import { EnterpriseNotifier } from "./notifier";

export interface SweepCandidate {
  pubkey: PublicKey;
  mint: PublicKey;
  balance: bigint;
  lamports: number;
}

export class SweeperEngine {
  private connection: Connection;
  private payer: Keypair;
  private notifier: EnterpriseNotifier;

  constructor(rpcUrl: string, payer: Keypair) {
    this.connection = new Connection(rpcUrl, "confirmed");
    this.payer = payer;
    this.notifier = new EnterpriseNotifier();
  }

  async findDustAccounts(maxDustLamports: bigint = BigInt(10_000_000)): Promise<SweepCandidate[]> {
    logger.info(`Scanning token accounts for owner: ${this.payer.publicKey.toBase58()}`);
    const accounts = await this.connection.getParsedTokenAccountsByOwner(
      this.payer.publicKey,
      { programId: TOKEN_PROGRAM_ID }
    );

    const sweepCandidates: SweepCandidate[] = [];

    for (const { pubkey, account } of accounts.value) {
      const parsedInfo = account.data.parsed.info;
      const balance = BigInt(parsedInfo.tokenAmount.amount);
      const mint = new PublicKey(parsedInfo.mint);
      const lamports = account.lamports;

      // Filter dust or zero balance
      if (balance <= maxDustLamports) {
        sweepCandidates.push({
          pubkey,
          mint,
          balance,
          lamports,
        });
      }
    }

    logger.info(`Found ${sweepCandidates.length} sweepable dust/inactive account(s).`);
    return sweepCandidates;
  }

  async executeBatchSweep(candidates: SweepCandidate[]): Promise<string[]> {
    if (candidates.length === 0) {
      logger.info("Zero accounts qualify for sweeping.");
      return [];
    }

    const signatures: string[] = [];
    const BATCH_SIZE = 5; // Safe instruction batch size avoiding Solana transaction size limits (1232 bytes)

    for (let i = 0; i < candidates.length; i += BATCH_SIZE) {
      const batch = candidates.slice(i, i + BATCH_SIZE);
      const tx = new Transaction();

      let estimatedReclaim = 0;

      for (const item of batch) {
        // 1. Burn dust if remaining
        if (item.balance > 0n) {
          tx.add(
            createBurnInstruction(
              item.pubkey,
              item.mint,
              this.payer.publicKey,
              item.balance
            )
          );
        }
        // 2. Close account and refund SOL rent
        tx.add(
          createCloseAccountInstruction(
            item.pubkey,
            this.payer.publicKey,
            this.payer.publicKey
          )
        );
        estimatedReclaim += item.lamports;
      }

      const latestBlockhash = await this.connection.getLatestBlockhash("confirmed");
      tx.recentBlockhash = latestBlockhash.blockhash;
      tx.feePayer = this.payer.publicKey;

      logger.info(`Executing sweep batch ${Math.floor(i / BATCH_SIZE) + 1}...`);
      const sig = await sendAndConfirmTransaction(this.connection, tx, [this.payer]);
      signatures.push(sig);
      logger.info(`Sweep Batch Executed! Signature: ${sig}`);

      await this.notifier.sendSlackDiscordAlert("Batch Dust Sweep Completed", `Reclaimed SOL rent for ${batch.length} accounts.`, {
        Signature: sig,
        AccountsClosed: batch.length,
        ReclaimedSOL: (estimatedReclaim / 1e9).toFixed(5),
      });
    }

    return signatures;
  }
}
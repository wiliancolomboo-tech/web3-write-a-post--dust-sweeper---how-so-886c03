import { Command } from "commander";
import { Keypair, PublicKey, Connection } from "@solana/web3.js";
import * as fs from "fs";
import { EnterpriseDustSweeperWorker } from "./sweep-and-quest-worker";

const program = new Command();

program
  .name("solana-dust-cli")
  .description("Enterprise Dust Sweeper & Social Quest XP Automation CLI")
  .version("1.0.0");

function loadKeypair(path: string): Keypair {
  const secret = JSON.parse(fs.readFileSync(path, "utf-8"));
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

program
  .command("scan")
  .description("Scan a wallet for sweepable token accounts")
  .requiredOption("-u, --user <pubkey>", "User public key to inspect")
  .requiredOption("-r, --rpc <url>", "Solana RPC URL", "https://api.devnet.solana.com")
  .action(async (options) => {
    const worker = new EnterpriseDustSweeperWorker({
      solanaRpcUrl: options.rpc,
      workerKeypair: Keypair.generate(),
      oracleKeypair: Keypair.generate(),
      programId: new PublicKey("Sweep11111111111111111111111111111111111111"),
      resendApiKey: "",
      notificationEmail: "",
      slackWebhookUrl: "",
    });

    const user = new PublicKey(options.user);
    const sweepable = await worker.discoverSweepableAccounts(user);
    console.log(`[INFO] Found ${sweepable.length} sweepable accounts for ${user.toBase58()}:`);
    sweepable.forEach((acc, i) => {
      console.log(`  [${i + 1}] ${acc.toBase58()} (Rent value: ~0.002039 SOL)`);
    });
    console.log(`[TOTAL RECOVERABLE] ~${(sweepable.length * 0.00203928).toFixed(6)} SOL`);
  });

program
  .command("issue-quest")
  .description("Simulate backend verification of social quest and print attestation")
  .requiredOption("-u, --user <pubkey>", "User public key")
  .requiredOption("-q, --quest <name>", "Quest identifier string (e.g. FOLLOW_ON_X)")
  .requiredOption("-x, --xp <amount>", "XP Reward amount", "250")
  .requiredOption("-k, --keypair <path>", "Oracle keypair path")
  .action(async (options) => {
    const oracleKeypair = loadKeypair(options.keypair);
    const worker = new EnterpriseDustSweeperWorker({
      solanaRpcUrl: "https://api.devnet.solana.com",
      workerKeypair: oracleKeypair,
      oracleKeypair: oracleKeypair,
      programId: new PublicKey("Sweep11111111111111111111111111111111111111"),
      resendApiKey: "",
      notificationEmail: "",
      slackWebhookUrl: "",
    });

    const user = new PublicKey(options.user);
    const attestation = worker.generateQuestAttestation(user, options.quest, parseInt(options.xp));
    console.log("[ORACLE ATTESTATION GENERATED]");
    console.log({
      user: user.toBase58(),
      questId: attestation.questId.toString("hex"),
      xpReward: attestation.xpReward,
      nonce: attestation.nonce.toString(),
      issuedAt: attestation.issuedAt.toString(),
      signature: attestation.signature.toString("hex"),
    });
  });

program.parse(process.argv);
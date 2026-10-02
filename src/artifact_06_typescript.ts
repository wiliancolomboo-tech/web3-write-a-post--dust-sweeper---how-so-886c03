import { Command } from "commander";
import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
  Ed25519Program,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  SystemProgram,
} from "@solana/web3.js";
import { Program, AnchorProvider, Wallet, Idl, BN } from "@coral-xyz/anchor";
import { TOKEN_PROGRAM_ID, AccountLayout } from "@solana/spl-token";
import axios from "axios";
import * as dotenv from "dotenv";
import * as fs from "fs";
import nacl from "tweetnacl";

dotenv.config();

const PROGRAM_ID = new PublicKey("SweepR8mQjH27Nq4gP5WbYyvVd1X8G4pLwK3qH7vZ5m");
const PROFILE_SEED = Buffer.from("user_profile");
const CONFIG_SEED = Buffer.from("global_config");

// Simplified IDL subset for Client execution
const IDL: Idl = {
  version: "0.1.0",
  name: "dust_sweeper_xp",
  instructions: [
    {
      name: "initializeUserProfile",
      accounts: [
        { name: "profile", isMut: true, isSigner: false },
        { name: "user", isMut: true, isSigner: true },
        { name: "systemProgram", isMut: false, isSigner: false },
      ],
      args: [],
    },
    {
      name: "sweepDustAndCloseAccount",
      accounts: [
        { name: "profile", isMut: true, isSigner: false },
        { name: "config", isMut: true, isSigner: false },
        { name: "tokenAccount", isMut: true, isSigner: false },
        { name: "tokenMint", isMut: true, isSigner: false },
        { name: "user", isMut: true, isSigner: true },
        { name: "owner", isMut: false, isSigner: false },
        { name: "tokenProgram", isMut: false, isSigner: false },
      ],
      args: [],
    },
    {
      name: "verifyAndClaimSocialQuest",
      accounts: [
        { name: "profile", isMut: true, isSigner: false },
        { name: "config", isMut: false, isSigner: false },
        { name: "user", isMut: true, isSigner: true },
        { name: "ixSysvar", isMut: false, isSigner: false },
      ],
      args: [
        { name: "questId", type: "u8" },
        { name: "xpReward", type: "u64" },
        { name: "nonce", type: "u64" },
        { name: "sigData", type: { array: ["u8", 64] } },
      ],
    },
  ],
};

function loadKeypair(path: string): Keypair {
  const secretKey = JSON.parse(fs.readFileSync(path, "utf-8"));
  return Keypair.fromSecretKey(new Uint8Array(secretKey));
}

async function sendEnterpriseNotification(webhookUrl: string, title: string, fields: Record<string, any>) {
  if (!webhookUrl) return;
  try {
    const payload = {
      embeds: [
        {
          title: `🧹 [Solana Dust Sweeper] ${title}`,
          color: 0x14f195, // Solana Green
          fields: Object.entries(fields).map(([name, value]) => ({
            name,
            value: String(value),
            inline: true,
          })),
          timestamp: new Date().toISOString(),
        },
      ],
    };
    await axios.post(webhookUrl, payload);
  } catch (err: any) {
    console.error("Failed to dispatch enterprise webhook notification:", err.message);
  }
}

const programCli = new Command();
programCli.version("1.0.0").description("Enterprise Solana Dust Sweeper and Social XP Agent");

programCli
  .command("sweep")
  .description("Scans wallet, burns dust, closes dormant accounts, and reclaims rent SOL")
  .option("-k, --keypair <path>", "Path to wallet keypair", process.env.WALLET_PATH || "./wallet.json")
  .option("-r, --rpc <url>", "RPC endpoint", process.env.SOLANA_RPC_URL || "https://api.mainnet-beta.solana.com")
  .option("-w, --webhook <url>", "Enterprise Webhook URL", process.env.ENTERPRISE_WEBHOOK_URL || "")
  .action(async (options) => {
    const connection = new Connection(options.rpc, "confirmed");
    const userWallet = loadKeypair(options.keypair);
    const provider = new AnchorProvider(connection, new Wallet(userWallet), { commitment: "confirmed" });
    const program = new Program(IDL, PROGRAM_ID, provider);

    console.log(`Analyzing dormant token accounts for wallet: ${userWallet.publicKey.toBase58()}`);

    const [profilePda] = PublicKey.findProgramAddressSync(
      [PROFILE_SEED, userWallet.publicKey.toBuffer()],
      PROGRAM_ID
    );
    const [configPda] = PublicKey.findProgramAddressSync([CONFIG_SEED], PROGRAM_ID);

    // Ensure Profile is initialized
    const profileInfo = await connection.getAccountInfo(profilePda);
    if (!profileInfo) {
      console.log("Initializing User Profile PDA...");
      await program.methods
        .initializeUserProfile()
        .accounts({
          profile: profilePda,
          user: userWallet.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      console.log("Profile PDA initialized.");
    }

    // Scan SPL Token Accounts
    const accounts = await connection.getTokenAccountsByOwner(userWallet.publicKey, {
      programId: TOKEN_PROGRAM_ID,
    });

    console.log(`Discovered ${accounts.value.length} SPL token accounts.`);
    let sweptCount = 0;
    let totalRentReclaimed = 0;

    for (const rawAccount of accounts.value) {
      const parsedData = AccountLayout.decode(rawAccount.account.data);
      const amount = parsedData.amount;
      const mintPubkey = new PublicKey(parsedData.mint);

      // Condition: Zero tokens or microscopic dust amount (< 1000 units)
      if (amount <= BigInt(1000)) {
        console.log(`Processing closure for account: ${rawAccount.pubkey.toBase58()} (Mint: ${mintPubkey.toBase58()})`);

        try {
          const tx = await program.methods
            .sweepDustAndCloseAccount()
            .accounts({
              profile: profilePda,
              config: configPda,
              tokenAccount: rawAccount.pubkey,
              tokenMint: mintPubkey,
              user: userWallet.publicKey,
              owner: userWallet.publicKey,
              tokenProgram: TOKEN_PROGRAM_ID,
            })
            .rpc();

          sweptCount++;
          totalRentReclaimed += rawAccount.account.lamports;
          console.log(`Closed successfully: Tx: ${tx}`);
        } catch (err: any) {
          console.error(`Failed to close ${rawAccount.pubkey.toBase58()}:`, err.message);
        }
      }
    }

    const solReclaimed = totalRentReclaimed / 1e9;
    console.log(`\n=== Sweep Operation Completed ===`);
    console.log(`Total Accounts Swept: ${sweptCount}`);
    console.log(`Total SOL Reclaimed: ${solReclaimed.toFixed(6)} SOL`);
    console.log(`Total XP Accrued: ${sweptCount * 50} XP`);

    await sendEnterpriseNotification(options.webhook, "Rent Sweep Cycle Completed", {
      "Wallet Address": userWallet.publicKey.toBase58(),
      "Accounts Closed": sweptCount,
      "Rent Reclaimed": `${solReclaimed.toFixed(6)} SOL`,
      "XP Earned": sweptCount * 50,
    });
  });

programCli
  .command("claim-quest")
  .description("Verifies off-chain social quest completion and claims on-chain XP with Ed25519 attestation")
  .requiredOption("-q, --quest-id <id>", "Quest ID [0-63]")
  .requiredOption("-x, --xp <reward>", "XP reward amount")
  .option("-k, --keypair <path>", "Path to user wallet keypair", process.env.WALLET_PATH || "./wallet.json")
  .option("-o, --oracle-keypair <path>", "Path to Oracle keypair (Attestation Signer)", process.env.ORACLE_PATH || "./oracle.json")
  .option("-r, --rpc <url>", "Solana RPC URL", process.env.SOLANA_RPC_URL || "https://api.mainnet-beta.solana.com")
  .option("-w, --webhook <url>", "Enterprise Webhook URL", process.env.ENTERPRISE_WEBHOOK_URL || "")
  .action(async (options) => {
    const connection = new Connection(options.rpc, "confirmed");
    const userWallet = loadKeypair(options.keypair);
    const oracleKeypair = loadKeypair(options.oracleKeypair);
    const provider = new AnchorProvider(connection, new Wallet(userWallet), { commitment: "confirmed" });
    const program = new Program(IDL, PROGRAM_ID, provider);

    const questId = parseInt(options.questId, 10);
    const xpReward = new BN(options.xp);
    const nonce = new BN(Date.now());

    const [profilePda] = PublicKey.findProgramAddressSync(
      [PROFILE_SEED, userWallet.publicKey.toBuffer()],
      PROGRAM_ID
    );
    const [configPda] = PublicKey.findProgramAddressSync([CONFIG_SEED], PROGRAM_ID);

    // Build Oracle Message: [User (32) | questId (1) | xpReward (8 LE) | nonce (8 LE)]
    const message = Buffer.concat([
      userWallet.publicKey.toBuffer(),
      Buffer.from([questId]),
      xpReward.toArrayLike(Buffer, "le", 8),
      nonce.toArrayLike(Buffer, "le", 8),
    ]);

    // Sign message with Ed25519 Oracle Keypair
    const signature = nacl.sign.detached(message, oracleKeypair.secretKey);

    // Build native Ed25519 Pre-instruction
    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracleKeypair.publicKey.toBytes(),
      message,
      signature,
    });

    // Build claim instruction
    const claimIx = await program.methods
      .verifyAndClaimSocialQuest(questId, xpReward, nonce, Array.from(signature))
      .accounts({
        profile: profilePda,
        config: configPda,
        user: userWallet.publicKey,
        ixSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    const sig = await sendAndConfirmTransaction(connection, tx, [userWallet]);

    console.log(`Quest ${questId} claimed successfully!`);
    console.log(`Transaction Signature: ${sig}`);

    await sendEnterpriseNotification(options.webhook, "Social Quest XP Claimed", {
      "User Pubkey": userWallet.publicKey.toBase58(),
      "Quest ID": questId,
      "XP Rewarded": xpReward.toString(),
      "Attestation Signer": oracleKeypair.publicKey.toBase58(),
    });
  });

programCli.parse(process.argv);
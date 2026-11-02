import {
    Connection,
    Keypair,
    PublicKey,
    Transaction,
    sendAndConfirmTransaction,
    Ed25519Program,
    TransactionInstruction,
    SystemProgram,
} from "@solana/web3.js";
import {
    TOKEN_PROGRAM_ID,
    createCloseAccountInstruction,
    createBurnInstruction,
} from "@solana/spl-token";
import * as nacl from "tweetnacl";
import axios from "axios";

export interface QuestVerificationRequest {
    userPubkey: string;
    questType: "X_FOLLOW" | "X_RETWEET" | "DISCORD_VERIFIED";
    questId: number;
    xpReward: number;
}

export interface EnterpriseNotificationPayload {
    user: string;
    totalSolRecovered: number;
    accountsClosed: number;
    xpEarned: number;
    txSignature: string;
}

export class SweepXPAgent {
    private connection: Connection;
    private oracleKeypair: Keypair;
    private programId: PublicKey;
    private resendApiKey?: string;
    private discordWebhookUrl?: string;

    constructor(
        rpcUrl: string,
        oracleSecretKey: Uint8Array,
        programId: string,
        discordWebhookUrl?: string,
        resendApiKey?: string
    ) {
        this.connection = new Connection(rpcUrl, "confirmed");
        this.oracleKeypair = Keypair.fromSecretKey(oracleSecretKey);
        this.programId = new PublicKey(programId);
        this.discordWebhookUrl = discordWebhookUrl;
        this.resendApiKey = resendApiKey;
    }

    /**
     * Scan user's wallet for empty or dust token accounts
     */
    async discoverDustAccounts(userPubkey: PublicKey) {
        const tokenAccounts = await this.connection.getParsedTokenAccountsByOwner(
            userPubkey,
            { programId: TOKEN_PROGRAM_ID }
        );

        const dustAccounts: Array<{
            pubkey: PublicKey;
            mint: PublicKey;
            amount: string;
            decimals: number;
            uiAmount: number;
        }> = [];

        for (const account of tokenAccounts.value) {
            const parsedInfo = account.account.data.parsed.info;
            const amount = parsedInfo.tokenAmount.amount;
            const decimals = parsedInfo.tokenAmount.decimals;
            const uiAmount = parsedInfo.tokenAmount.uiAmount || 0;

            // Accounts with 0 balance or negligible dust
            if (uiAmount === 0 || uiAmount < 0.0001) {
                dustAccounts.push({
                    pubkey: account.pubkey,
                    mint: new PublicKey(parsedInfo.mint),
                    amount,
                    decimals,
                    uiAmount,
                });
            }
        }

        return dustAccounts;
    }

    /**
     * Creates an Ed25519 cryptographic attestation for validated social quests
     */
    generateQuestProof(
        user: PublicKey,
        questId: bigint,
        xpAmount: number,
        nonce: bigint
    ): { signature: Uint8Array; message: Uint8Array } {
        const message = new Uint8Array(52);
        const dataView = new DataView(message.buffer);

        message.set(user.toBytes(), 0);
        dataView.setBigUint64(32, questId, true);
        dataView.setUint32(40, xpAmount, true);
        dataView.setBigUint64(44, nonce, true);

        const signature = nacl.sign.detached(message, this.oracleKeypair.secretKey);
        return { signature, message };
    }

    /**
     * Creates the pre-instruction to verify oracle signature on-chain
     */
    createEd25519Instruction(
        message: Uint8Array,
        signature: Uint8Array
    ): TransactionInstruction {
        return Ed25519Program.createInstructionWithPublicKey({
            publicKey: this.oracleKeypair.publicKey.toBytes(),
            message,
            signature,
        });
    }

    /**
     * Dispatch multi-channel notifications to Discord and Resend (Email)
     */
    async sendEnterpriseNotification(payload: EnterpriseNotificationPayload) {
        // 1. Dispatch Discord Alert
        if (this.discordWebhookUrl) {
            try {
                await axios.post(this.discordWebhookUrl, {
                    embeds: [
                        {
                            title: "🧹 Dust Sweeper Execution Confirmed",
                            color: 0x14f195, // Solana Green
                            fields: [
                                { name: "User Wallet", value: `\`${payload.user}\``, inline: false },
                                { name: "SOL Recovered", value: `${payload.totalSolRecovered.toFixed(6)} SOL`, inline: true },
                                { name: "ATAs Closed", value: `${payload.accountsClosed}`, inline: true },
                                { name: "XP Awarded", value: `+${payload.xpEarned} XP`, inline: true },
                                { name: "Explorer Link", value: `[SolanaFM](https://solana.fm/tx/${payload.txSignature})`, inline: false },
                            ],
                            footer: { text: "SweepXP Enterprise Automation Engine" },
                            timestamp: new Date().toISOString(),
                        },
                    ],
                });
            } catch (err: any) {
                console.error("[Enterprise-Notify] Discord dispatch failed:", err.message);
            }
        }

        // 2. Dispatch Email via Resend if API key is provided
        if (this.resendApiKey) {
            try {
                await axios.post(
                    "https://api.resend.com/emails",
                    {
                        from: "SweepXP Alerts <alerts@sweepxp.solana>",
                        to: ["admin@enterprise-solana.com"],
                        subject: `[SweepXP] User Claimed ${payload.xpEarned} XP & Cleaned ${payload.accountsClosed} ATAs`,
                        html: `<h2>SweepXP Receipt</h2>
                               <p><strong>Wallet:</strong> ${payload.user}</p>
                               <p><strong>Rent Recovered:</strong> ${payload.totalSolRecovered} SOL</p>
                               <p><strong>XP Minted:</strong> ${payload.xpEarned}</p>
                               <p><strong>Tx:</strong> <a href="https://solscan.io/tx/${payload.txSignature}">${payload.txSignature}</a></p>`,
                    },
                    {
                        headers: {
                            Authorization: `Bearer ${this.resendApiKey}`,
                            "Content-Type": "application/json",
                        },
                    }
                );
            } catch (err: any) {
                console.error("[Enterprise-Notify] Resend dispatch failed:", err.message);
            }
        }
    }
}

// Enterprise CLI Entrypoint
async function main() {
    const rpcUrl = process.env.SOLANA_RPC_URL || "https://api.devnet.solana.com";
    const oracleSecret = process.env.ORACLE_PRIVATE_KEY
        ? Uint8Array.from(JSON.parse(process.env.ORACLE_PRIVATE_KEY))
        : Keypair.generate().secretKey;
    const programId = process.env.PROGRAM_ID || "SwpXP111111111111111111111111111111111111111";

    const agent = new SweepXPAgent(
        rpcUrl,
        oracleSecret,
        programId,
        process.env.DISCORD_WEBHOOK_URL,
        process.env.RESEND_API_KEY
    );

    console.log("SweepXP Enterprise Worker initialized.");
    console.log(`Connected to RPC: ${rpcUrl}`);
}

if (require.main === module) {
    main().catch(console.error);
}
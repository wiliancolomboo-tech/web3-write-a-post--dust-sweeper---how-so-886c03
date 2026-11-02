import axios from 'axios';

export interface TelemetryReport {
  userAddress: string;
  accountsClosed: number;
  reclaimedSol: number;
  xpEarned: number;
  txSignature: string;
}

export async function sendEnterpriseWebhook(report: TelemetryReport): Promise<void> {
  const webhookUrl = process.env.DISCORD_WEBHOOK_URL;
  if (!webhookUrl) {
    console.log('[Telemetry] No DISCORD_WEBHOOK_URL configured. Skipping webhook push.');
    return;
  }

  const payload = {
    embeds: [
      {
        title: "⚡ Solana Dust Swept & Social XP Awarded",
        color: 0x14F195, // Solana Green
        fields: [
          { name: "Wallet", value: `\`${report.userAddress}\``, inline: false },
          { name: "Accounts Closed", value: `${report.accountsClosed}`, inline: true },
          { name: "Reclaimed Rent", value: `${report.reclaimedSol.toFixed(6)} SOL`, inline: true },
          { name: "Total XP Earned", value: `+${report.xpEarned} XP`, inline: true },
          { name: "Transaction Hash", value: `[SolanaFM](https://solana.fm/tx/${report.txSignature})`, inline: false }
        ],
        footer: {
          text: "Dust Sweeper & Social Quests Engine v1.0.0"
        },
        timestamp: new Date().toISOString()
      }
    ]
  };

  try {
    await axios.post(webhookUrl, payload, { timeout: 5000 });
    console.log('[Telemetry] Enterprise notification sent successfully.');
  } catch (err: any) {
    console.error(`[Telemetry Error] Failed to dispatch webhook: ${err.message}`);
  }
}
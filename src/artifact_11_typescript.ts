import express from "express";
import { Connection } from "@solana/web3.js";

const app = express();
const connection = new Connection(process.env.SOLANA_RPC_URL || "https://api.mainnet-beta.solana.com");

app.get("/health", async (_req, res) => {
  try {
    const epochInfo = await connection.getEpochInfo();
    return res.status(200).json({
      status: "HEALTHY",
      currentEpoch: epochInfo.epoch,
      slotIndex: epochInfo.slotIndex,
      uptimeSeconds: process.uptime(),
      timestamp: new Date().toISOString(),
    });
  } catch (err: any) {
    return res.status(503).json({
      status: "DEGRADED",
      error: err.message,
    });
  }
});

const PORT = process.env.PORT || 8080;
app.listen(PORT, () => console.log(`SolSweep Gateway running on :${PORT}`));
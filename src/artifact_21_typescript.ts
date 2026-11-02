// healthcheck.ts
import http from "http";
import { Connection } from "@solana/web3.js";

const connection = new Connection(process.env.SOLANA_RPC_URL || "https://api.devnet.solana.com");

const server = http.createServer(async (req, res) => {
  if (req.url === "/healthz") {
    try {
      const slot = await connection.getSlot();
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "healthy", currentSlot: slot, timestamp: Date.now() }));
    } catch (e: any) {
      res.writeHead(503, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "degraded", error: e.message }));
    }
  } else {
    res.writeHead(404);
    res.end();
  }
});

server.listen(8080, () => console.log("Telemetry running on port 8080"));
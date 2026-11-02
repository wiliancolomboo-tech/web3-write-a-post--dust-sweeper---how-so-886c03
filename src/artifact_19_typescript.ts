import * as http from "http";

const PORT = process.env.PORT || 8080;

http.createServer(async (req, res) => {
  if (req.url === "/health" && req.method === "GET") {
    try {
      const connection = new Connection(RPC_URL, "confirmed");
      const epochInfo = await connection.getEpochInfo();
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          status: "healthy",
          uptime: process.uptime(),
          currentEpoch: epochInfo.epoch,
          slotIndex: epochInfo.slotIndex,
          timestamp: new Date().toISOString(),
        })
      );
    } catch (err: any) {
      res.writeHead(503, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "unhealthy", error: err.message }));
    }
  } else {
    res.writeHead(404);
    res.end();
  }
}).listen(PORT, () => {
  console.log(`[Telemetry] Healthcheck server live on :${PORT}/health`);
});
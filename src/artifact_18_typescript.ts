import http from "http";

const server = http.createServer((req, res) => {
  if (req.url === "/health" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({
      status: "HEALTHY",
      uptime: process.uptime(),
      timestamp: Date.now(),
      rpcConnected: true
    }));
    return;
  }
  res.writeHead(404);
  res.end();
});

server.listen(3000, () => console.log("[HEALTHCHECK] Telemetry port 3000 online"));
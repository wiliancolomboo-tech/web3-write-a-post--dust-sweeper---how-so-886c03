import http from "http";

const server = http.createServer((req, res) => {
  if (req.url === "/health" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(
      JSON.stringify({
        status: "HEALTHY",
        uptime: process.uptime(),
        timestamp: new Date().toISOString(),
        network: "solana-mainnet",
      })
    );
    return;
  }
  res.writeHead(404);
  res.end();
});

server.listen(process.env.PORT || 8080);
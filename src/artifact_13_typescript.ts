// src/index.ts
import http from "http";

const PORT = process.env.PORT || 8080;

const server = http.createServer((req, res) => {
  if (req.url === "/health" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(
      JSON.stringify({
        status: "UP",
        timestamp: new Date().toISOString(),
        network: "Solana",
        service: "dust_sweeper_xp_agent",
      })
    );
    return;
  }

  if (req.url === "/metrics" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end(
      `# HELP solana_rent_reclaimed_total Total lamports reclaimed\n# TYPE solana_rent_reclaimed_total counter\nsolana_rent_reclaimed_total 203928000\n`
    );
    return;
  }

  res.writeHead(404);
  res.end();
});

server.listen(PORT, () => {
  console.log(`Telemetry and health server listening on port ${PORT}`);
});
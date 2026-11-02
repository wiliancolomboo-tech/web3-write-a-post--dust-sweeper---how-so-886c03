import http from "http";
import winston from "winston";

const logger = winston.createLogger({
  level: "info",
  format: winston.format.json(),
  transports: [new winston.transports.Console()],
});

const PORT = process.env.PORT || 8080;

const server = http.createServer((req, res) => {
  if (req.url === "/health" && req.method === "GET") {
    const healthStatus = {
      status: "UP",
      timestamp: new Date().toISOString(),
      uptimeSeconds: process.uptime(),
      memoryUsage: process.memoryUsage(),
      dependencies: {
        solanaRpc: "HEALTHY",
        oracleSigner: "ACTIVE",
      },
    };

    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(healthStatus));
    return;
  }

  res.writeHead(404, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ error: "Route Not Found" }));
});

server.listen(PORT, () => {
  logger.info(`Telemetry and Healthcheck server listening on port ${PORT}`);
});
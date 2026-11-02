import Fastify from "fastify";
import { SolSweepClient } from "./index";

const fastify = Fastify({ logger: true });
const client = new SolSweepClient();

fastify.get("/health", async (request, reply) => {
  try {
    const version = await client.connection.getVersion();
    return {
      status: "HEALTHY",
      uptime: process.uptime(),
      solanaCoreVersion: version["solana-core"],
      oraclePublicKey: client.oracleKeypair.publicKey.toBase58(),
      timestamp: new Date().toISOString()
    };
  } catch (error) {
    reply.status(503);
    return { status: "UNHEALTHY", reason: (error as Error).message };
  }
});

fastify.post("/api/v1/sweep", async (request, reply) => {
  await client.scanAndSweepDust();
  return { status: "QUEUED_AND_EXECUTED" };
});

const start = async () => {
  try {
    const port = Number(process.env.PORT) || 8080;
    await fastify.listen({ port, host: "0.0.0.0" });
    console.log(`Enterprise Agent Server listening on http://0.0.0.0:${port}`);
  } catch (err) {
    fastify.log.error(err);
    process.exit(1);
  }
};

start();
import { Hono } from "hono";
import { cors } from "hono/cors";
import { serve } from "@hono/node-server";
import registerRoute from "./routes/register.js";
import sigHashRoute from "./routes/sig-hash.js";
import simpleSendRoute from "./routes/simple-send.js";
import sponsoredSendRoute from "./routes/sponsored-send.js";
import batchOpsRoute from "./routes/batch-ops.js";
import deployExecuteRoute from "./routes/deploy-execute.js";
import { ensureFactoryInitialized } from "./dev-account.js";

const app = new Hono();

// CORS for all origins (demo)
app.use("/*", cors());

// Mount all routes under /api
app.route("/api", registerRoute);
app.route("/api", sigHashRoute);
app.route("/api", simpleSendRoute);
app.route("/api", sponsoredSendRoute);
app.route("/api", batchOpsRoute);
app.route("/api", deployExecuteRoute);

// Health check
app.get("/health", (c) => c.json({ status: "ok" }));

const port = parseInt(process.env.PORT ?? "3000", 10);

console.log(`[eip8141-backend] Starting on port ${port}`);
console.log(`[eip8141-backend] RPC_URL=${process.env.RPC_URL ?? "http://localhost:8545"}`);

// Initialize factory before accepting requests
async function start() {
  try {
    await ensureFactoryInitialized();
  } catch (err) {
    console.error(`[eip8141-backend] Factory init failed: ${err}`);
    console.error("[eip8141-backend] Make sure ethrex is running and 'make genesis' was run");
    process.exit(1);
  }

  serve({ fetch: app.fetch, port }, () => {
    console.log(`[eip8141-backend] Listening on http://localhost:${port}`);
  });
}

start();

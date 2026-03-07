const express = require("express");
const cors = require("cors");
const path = require("path");

const deploymentRoutes = require("./routes/deployments");
const hostRoutes = require("./routes/hosts");
const fsRoutes = require("./routes/fs");

const app = express();
const PORT = process.env.LOCAL_SERVER_PORT || 5002;

// Middleware
app.use(cors({
  origin: true,  // Allow all origins (localhost-only server)
  credentials: true,
}));
app.use(express.json());

// Static web UI (no cache during development)
app.use(express.static(path.join(__dirname, "public"), {
  etag: false,
  maxAge: 0,
  setHeaders: (res) => {
    res.set("Cache-Control", "no-store, no-cache, must-revalidate");
  },
}));

// API Routes
app.use("/api/deployments", deploymentRoutes);
app.use("/api/hosts", hostRoutes);
app.use("/api/fs", fsRoutes);

// Store proxy — fetch programs from Platform API, fallback to defaults
app.get("/api/store/programs", async (req, res) => {
  const PLATFORM_API = process.env.PLATFORM_API_URL || "https://tokamak-platform.web.app";
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);
    const resp = await fetch(`${PLATFORM_API}/api/store/programs`, { signal: controller.signal });
    clearTimeout(timeout);
    if (resp.ok) {
      const data = await resp.json();
      return res.json(data);
    }
  } catch (_) {
    // Platform unreachable, use defaults
  }
  // Fallback — 2 official programs (only ones with ready implementations)
  res.json([
    { id: "evm-l2", program_id: "evm-l2", name: "EVM L2", description: "Default Ethereum execution environment. Full EVM compatibility for general-purpose L2 chains.", author: "Tokamak", category: "defi", tags: ["evm", "defi"], is_official: true },
    { id: "zk-dex", program_id: "zk-dex", name: "ZK-DEX", description: "Decentralized exchange circuits optimized for on-chain order matching and settlement.", author: "Tokamak", category: "defi", tags: ["zk", "defi", "exchange"], is_official: true },
  ]);
});

// Recovery: detect stuck deployments on server start
const { recoverStuckDeployments } = require("./lib/deployment-engine");
recoverStuckDeployments();

// Open URL in system browser (for Tauri WebviewWindow where window.open is blocked)
app.post("/api/open-url", (req, res) => {
  const { url } = req.body;
  if (!url || typeof url !== "string") return res.status(400).json({ error: "url required" });
  // Only allow http/https URLs on localhost or known domains
  if (!url.startsWith("http://127.0.0.1") && !url.startsWith("http://localhost") && !url.startsWith("https://")) {
    return res.status(400).json({ error: "Invalid URL" });
  }
  const { exec } = require("child_process");
  const escaped = url.replace(/"/g, '\\"');
  const platform = process.platform;
  const cmd = platform === "win32" ? `start "" "${escaped}"`
    : platform === "darwin" ? `open "${escaped}"`
    : `xdg-open "${escaped}"`;
  exec(cmd, (err) => {
    if (err) return res.status(500).json({ error: err.message });
    res.json({ ok: true });
  });
});

// Health check
app.get("/api/health", (req, res) => {
  res.json({ status: "ok", version: "0.1.0", type: "local-server" });
});

// Error handler
app.use((err, req, res, _next) => {
  console.error("Unhandled error:", err);
  res.status(500).json({ error: "Internal server error" });
});

// Bind to localhost only (security: no external access)
if (require.main === module) {
  app.listen(PORT, "127.0.0.1", () => {
    console.log(`Tokamak local server running on http://127.0.0.1:${PORT}`);
  });
}

module.exports = app;

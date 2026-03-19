const express = require("express");
const cors = require("cors");
const path = require("path");

const deploymentRoutes = require("./routes/deployments");
const hostRoutes = require("./routes/hosts");
const fsRoutes = require("./routes/fs");
const keychainRoutes = require("./routes/keychain");

const app = express();
const PORT = process.env.LOCAL_SERVER_PORT || 5002;

// Middleware — restrict CORS to Tauri dev/prod origins (localhost-only server)
const ALLOWED_ORIGINS = [
  "tauri://localhost",        // Tauri production (macOS/Linux)
  "https://tauri.localhost",  // Tauri production (Windows)
  "http://localhost:1420",    // Tauri dev (Vite)
  "http://127.0.0.1:1420",
  "http://localhost:5173",    // Vite dev (default port)
  "http://127.0.0.1:5173",
  "http://localhost:5002",    // Self (web UI)
  "http://127.0.0.1:5002",
];
app.use(cors({
  origin: (origin, cb) => {
    // Allow requests with no origin (same-origin, curl, Tauri webview)
    if (!origin || ALLOWED_ORIGINS.includes(origin)) return cb(null, true);
    cb(new Error("CORS not allowed"));
  },
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
app.use("/api/keychain", keychainRoutes);

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
  // Fallback — official programs per stack
  res.json([
    { id: "evm-l2", program_id: "evm-l2", name: "EVM L2", description: "Default Ethereum execution environment. Full EVM compatibility for general-purpose L2 chains.", author: "Tokamak", category: "defi", tags: ["evm", "defi"], is_official: true, stack: "ethrex" },
    { id: "zk-dex", program_id: "zk-dex", name: "ZK-DEX", description: "Decentralized exchange circuits optimized for on-chain order matching and settlement.", author: "Tokamak", category: "defi", tags: ["zk", "defi", "exchange"], is_official: true, stack: "ethrex" },
    { id: "thanos-l2", program_id: "thanos-l2", name: "Thanos L2", description: "OP Stack-based Optimistic Rollup. Fault proof secured, fully EVM compatible L2 chain.", author: "Tokamak", category: "defi", tags: ["optimism", "op-stack", "defi"], is_official: true, stack: "thanos" },
  ]);
});

// Recovery: detect stuck deployments on server start
const { recoverStuckDeployments } = require("./lib/deployment-engine");
recoverStuckDeployments().catch(e => console.error("[recovery] Error:", e.message));

// Open URL in system browser (for Tauri WebviewWindow where window.open is blocked)
app.post("/api/open-url", (req, res) => {
  const { url } = req.body;
  if (!url || typeof url !== "string") return res.status(400).json({ error: "url required" });

  const { exec } = require("child_process");

  // Special case: register deployer key via macOS native secure dialogs
  // Private key never touches the web UI — entered only in OS-level dialog
  if (url === "keychain-register") {
    if (process.platform !== "darwin") return res.status(400).json({ error: "macOS only" });
    const script = `
      set keyName to text returned of (display dialog "Enter a name for this deployer key:" default answer "sepolia-deployer" with title "Tokamak Keychain" buttons {"Cancel", "Next"} default button "Next")
      set keyValue to text returned of (display dialog "Enter the deployer private key (0x...):" default answer "" with hidden answer with title "Tokamak Keychain" buttons {"Cancel", "Save"} default button "Save")
      do shell script "security add-generic-password -a " & quoted form of keyName & " -s tokamak-appchain -w " & quoted form of keyValue & " -U"
      return keyName
    `;
    exec(`osascript -e '${script.replace(/'/g, "'\\''")}'`, (err, stdout) => {
      if (err) {
        if (err.message.includes("-128")) return res.json({ ok: false, cancelled: true });
        return res.status(500).json({ error: err.message });
      }
      res.json({ ok: true, keyName: stdout.trim() });
    });
    return;
  }

  // Validate URL: only http/https on localhost or known Tokamak domains
  let parsed;
  try { parsed = new URL(url); } catch { return res.status(400).json({ error: "Invalid URL" }); }
  if (!["http:", "https:"].includes(parsed.protocol)) {
    return res.status(400).json({ error: "Invalid URL protocol" });
  }
  const ALLOWED_HOSTNAMES = [
    "localhost", "127.0.0.1", "0.0.0.0",
    "tokamak.network", "sepolia.etherscan.io", "etherscan.io",
    "holesky.etherscan.io",
  ];
  const isAllowed = ALLOWED_HOSTNAMES.some(
    (h) => parsed.hostname === h || parsed.hostname.endsWith("." + h)
  );
  if (!isAllowed) {
    return res.status(403).json({ error: "Hostname not in allowlist" });
  }

  const { execFile } = require("child_process");
  const opener = process.platform === "win32" ? "cmd"
    : process.platform === "darwin" ? "open" : "xdg-open";
  const args = process.platform === "win32" ? ["/c", "start", "", url] : [url];
  execFile(opener, args, (err) => {
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

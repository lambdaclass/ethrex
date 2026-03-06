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
  origin: [
    "http://localhost:3002",
    "http://127.0.0.1:3002",
    "http://localhost:1420",  // Tauri dev server
    "http://127.0.0.1:1420",
    "tauri://localhost",      // Tauri production
  ],
  credentials: true,
}));
app.use(express.json());

// Static web UI
app.use(express.static(path.join(__dirname, "public")));

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
  // Fallback default programs
  res.json([
    { id: "default-erc20", name: "ERC-20 Token", description: "Deploy a standard ERC-20 token on your L2", author: "Tokamak", tags: ["token", "defi"] },
    { id: "default-nft", name: "NFT Collection", description: "Launch an NFT collection with minting capabilities", author: "Tokamak", tags: ["nft", "collectible"] },
    { id: "default-dex", name: "DEX (AMM)", description: "Automated Market Maker decentralized exchange", author: "Tokamak", tags: ["defi", "exchange"] },
    { id: "default-bridge", name: "Token Bridge", description: "Bridge tokens between L1 and your L2 chain", author: "Tokamak", tags: ["bridge", "infra"] },
    { id: "default-blank", name: "Blank Chain", description: "Empty L2 chain — deploy your own contracts later", author: "Tokamak", tags: ["custom"] },
  ]);
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

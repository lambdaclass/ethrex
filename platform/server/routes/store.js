const express = require("express");
const router = express.Router();

const { getActivePrograms, getProgramById, getCategories } = require("../db/programs");
const { getActiveDeployments, getActiveDeploymentById } = require("../db/deployments");

// GET /api/store/programs — public program listing
router.get("/programs", (req, res) => {
  try {
    const { category, search, limit, offset } = req.query;
    const programs = getActivePrograms({
      category,
      search,
      limit: parseInt(limit) || 50,
      offset: parseInt(offset) || 0,
    });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/programs/:id — program detail
router.get("/programs/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.status !== "active") {
      return res.status(404).json({ error: "Program not found" });
    }
    res.json({ program });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/categories — category list
router.get("/categories", (req, res) => {
  try {
    const categories = getCategories();
    res.json({ categories });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/featured — featured programs (top by usage)
router.get("/featured", (req, res) => {
  try {
    const programs = getActivePrograms({ limit: 6 });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/appchains — public Open Appchain listing (Showroom)
router.get("/appchains", (req, res) => {
  try {
    const { search, limit, offset } = req.query;
    const appchains = getActiveDeployments({
      search,
      limit: parseInt(limit) || 50,
      offset: parseInt(offset) || 0,
    });
    res.json({ appchains });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/appchains/:id — public appchain detail (Showroom)
router.get("/appchains/:id", (req, res) => {
  try {
    const appchain = getActiveDeploymentById(req.params.id);
    if (!appchain) {
      return res.status(404).json({ error: "Appchain not found" });
    }
    let screenshots = [];
    let social_links = {};
    try { screenshots = appchain.screenshots ? JSON.parse(appchain.screenshots) : []; } catch { /* ignore */ }
    try { social_links = appchain.social_links ? JSON.parse(appchain.social_links) : {}; } catch { /* ignore */ }
    res.json({
      appchain: { ...appchain, screenshots, social_links },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/store/appchains/:id/rpc-proxy — L2 RPC proxy (CORS bypass)
router.post("/appchains/:id/rpc-proxy", async (req, res) => {
  try {
    const appchain = getActiveDeploymentById(req.params.id);
    if (!appchain || !appchain.rpc_url) {
      return res.status(404).json({ error: "Appchain not found or no RPC URL" });
    }

    // SSRF protection: only allow http(s) URLs, block private/internal IPs
    try {
      const rpcUrl = new URL(appchain.rpc_url);
      if (!["http:", "https:"].includes(rpcUrl.protocol)) {
        return res.status(400).json({ error: "Invalid RPC URL protocol" });
      }
      const host = rpcUrl.hostname;
      if (host === "localhost" || host === "127.0.0.1" || host === "::1" ||
          host.startsWith("10.") || host.startsWith("192.168.") ||
          host.startsWith("169.254.") || host.endsWith(".internal") ||
          /^172\.(1[6-9]|2\d|3[01])\./.test(host)) {
        return res.status(400).json({ error: "RPC URL cannot point to internal addresses" });
      }
    } catch {
      return res.status(400).json({ error: "Invalid RPC URL" });
    }

    const allowedMethods = [
      "eth_blockNumber", "eth_chainId", "eth_gasPrice",
      "ethrex_batchNumber", "ethrex_metadata", "net_version",
    ];

    const { method, params } = req.body;
    if (!method || !allowedMethods.includes(method)) {
      return res.status(400).json({ error: "Method not allowed" });
    }

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);

    const response = await fetch(appchain.rpc_url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: params || [] }),
      signal: controller.signal,
    });

    clearTimeout(timeout);
    const data = await response.json();
    res.json(data);
  } catch (e) {
    console.error(`[rpc-proxy] Error proxying to ${req.params.id}:`, e.message);
    res.status(502).json({ error: "L2 node unreachable" });
  }
});

module.exports = router;

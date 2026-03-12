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
    res.json({
      appchain: {
        ...appchain,
        screenshots: appchain.screenshots ? JSON.parse(appchain.screenshots) : [],
        social_links: appchain.social_links ? JSON.parse(appchain.social_links) : {},
      },
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

    const allowedMethods = [
      "eth_blockNumber", "eth_chainId", "eth_gasPrice",
      "ethrex_batchNumber", "net_version",
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
    res.status(502).json({ error: "L2 node unreachable", detail: e.message });
  }
});

module.exports = router;

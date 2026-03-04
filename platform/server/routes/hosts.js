const express = require("express");
const router = express.Router();

const { requireAuth } = require("../middleware/auth");
const { createHost, getHostsByUser, getHostById, updateHost, deleteHost } = require("../db/hosts");
const { testConnection } = require("../lib/docker-remote");

router.use(requireAuth);

// POST /api/hosts — add a remote host
router.post("/", (req, res) => {
  try {
    const { name, hostname, port, username, authMethod, privateKey } = req.body;
    if (!name || !hostname || !username) {
      return res.status(400).json({ error: "name, hostname, and username are required" });
    }
    if (authMethod === "key" && !privateKey) {
      return res.status(400).json({ error: "privateKey is required for key authentication" });
    }

    const host = createHost({
      userId: req.user.id,
      name: name.trim(),
      hostname: hostname.trim(),
      port: port || 22,
      username: username.trim(),
      authMethod: authMethod || "key",
      privateKey: privateKey || null,
    });

    // Return without private_key for security
    const { private_key, ...safeHost } = host;
    res.status(201).json({ host: safeHost });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/hosts — list my hosts
router.get("/", (req, res) => {
  try {
    const hosts = getHostsByUser(req.user.id);
    res.json({ hosts });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/hosts/:id — get host detail
router.get("/:id", (req, res) => {
  try {
    const host = getHostById(req.params.id);
    if (!host || host.user_id !== req.user.id) {
      return res.status(404).json({ error: "Host not found" });
    }
    const { private_key, ...safeHost } = host;
    res.json({ host: safeHost });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/hosts/:id/test — test SSH + Docker connection
router.post("/:id/test", async (req, res) => {
  try {
    const host = getHostById(req.params.id);
    if (!host || host.user_id !== req.user.id) {
      return res.status(404).json({ error: "Host not found" });
    }

    const result = await testConnection(host);

    // Update host status
    updateHost(host.id, {
      status: result.ok && result.docker ? "active" : result.ok ? "no_docker" : "error",
      last_tested: Date.now(),
    });

    res.json(result);
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/hosts/:id — update host config
router.put("/:id", (req, res) => {
  try {
    const host = getHostById(req.params.id);
    if (!host || host.user_id !== req.user.id) {
      return res.status(404).json({ error: "Host not found" });
    }
    const updated = updateHost(req.params.id, req.body);
    const { private_key, ...safeHost } = updated;
    res.json({ host: safeHost });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/hosts/:id — remove host
router.delete("/:id", (req, res) => {
  try {
    const host = getHostById(req.params.id);
    if (!host || host.user_id !== req.user.id) {
      return res.status(404).json({ error: "Host not found" });
    }
    deleteHost(req.params.id);
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

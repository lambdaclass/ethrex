const express = require("express");
const router = express.Router();

const { createHost, getAllHosts, getHostById, updateHost, deleteHost } = require("../db/hosts");
const { testConnection } = require("../lib/docker-remote");

// POST /api/hosts — add a remote host
// Security: accepts a file path to the SSH private key (privateKeyPath) instead
// of raw key content, avoiding credential exposure in HTTP requests and SQLite DB.
// The key is read from the filesystem only when needed for SSH connections.
router.post("/", (req, res) => {
  try {
    const { name, hostname, port, username, authMethod, privateKeyPath } = req.body;
    if (!name || !hostname || !username) {
      return res.status(400).json({ error: "name, hostname, and username are required" });
    }
    if (authMethod === "key" && !privateKeyPath) {
      return res.status(400).json({ error: "privateKeyPath is required for key authentication" });
    }

    // Validate key path exists (if provided)
    const path = require("path");
    const fs = require("fs");
    if (privateKeyPath) {
      const resolved = path.resolve(privateKeyPath);
      if (!fs.existsSync(resolved)) {
        return res.status(400).json({ error: "SSH key file not found at specified path" });
      }
    }

    const host = createHost({
      name: name.trim(),
      hostname: hostname.trim(),
      port: port || 22,
      username: username.trim(),
      authMethod: authMethod || "key",
      privateKey: privateKeyPath ? path.resolve(privateKeyPath) : null,
    });

    const { private_key, ...safeHost } = host;
    res.status(201).json({ host: safeHost });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/hosts — list all hosts
router.get("/", (req, res) => {
  try {
    const hosts = getAllHosts();
    res.json({ hosts });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/hosts/:id — get host detail
router.get("/:id", (req, res) => {
  try {
    const host = getHostById(req.params.id);
    if (!host) {
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
    if (!host) {
      return res.status(404).json({ error: "Host not found" });
    }

    const result = await testConnection(host);

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
    if (!host) {
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
    if (!host) {
      return res.status(404).json({ error: "Host not found" });
    }
    deleteHost(req.params.id);
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

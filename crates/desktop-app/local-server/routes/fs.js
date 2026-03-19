const express = require("express");
const router = express.Router();
const fs = require("fs");
const path = require("path");
const os = require("os");

// GET /api/fs/browse?path=/some/dir — list directories (restricted to home dir)
router.get("/browse", (req, res) => {
  try {
    const homeDir = os.homedir();
    const dirPath = req.query.path || homeDir;
    const resolved = path.resolve(dirPath);

    // Security: restrict browsing to home directory and below.
    // Use boundary-aware check to prevent prefix-match bypass
    // (e.g., /Users/al matching /Users/alice).
    if (resolved !== homeDir && !resolved.startsWith(homeDir + path.sep)) {
      return res.status(403).json({ error: "Access denied: path must be within home directory" });
    }

    if (!fs.existsSync(resolved)) {
      return res.status(404).json({ error: "Directory not found" });
    }

    const stat = fs.statSync(resolved);
    if (!stat.isDirectory()) {
      return res.status(400).json({ error: "Not a directory" });
    }

    const entries = fs.readdirSync(resolved, { withFileTypes: true });
    const dirs = entries
      .filter((e) => e.isDirectory() && !e.name.startsWith("."))
      .map((e) => ({
        name: e.name,
        path: path.join(resolved, e.name),
      }))
      .sort((a, b) => a.name.localeCompare(b.name));

    const parent = path.dirname(resolved);

    res.json({
      current: resolved,
      parent: parent !== resolved ? parent : null,
      dirs,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

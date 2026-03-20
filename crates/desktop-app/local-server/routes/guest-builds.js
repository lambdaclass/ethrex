/**
 * Guest Program Build API
 *
 * POST /api/guest-builds          — Start a new build
 * GET  /api/guest-builds/:id      — Get build status
 * GET  /api/guest-builds/:id/elf  — Download ELF binary
 * GET  /api/guest-builds/:id/logs — Get build logs (SSE stream)
 */

const express = require("express");
const crypto = require("crypto");
const { buildGuestProgram, getBuild, getAllBuilds, getElfBuffer, deleteBuild } = require("../lib/guest-builder");

const router = express.Router();

// Active SSE connections per build
const sseClients = new Map(); // buildId → Set<res>

/**
 * GET /api/guest-builds
 * returns: { builds: [...] }
 */
router.get("/", (req, res) => {
  res.json({ builds: getAllBuilds() });
});

/**
 * POST /api/guest-builds
 * body: { programName: string, sourceCode: string }
 * returns: { buildId: string }
 */
router.post("/", async (req, res) => {
  const { programName, sourceCode } = req.body;

  if (!programName || typeof programName !== "string") {
    return res.status(400).json({ error: "programName is required" });
  }
  if (!sourceCode || typeof sourceCode !== "string") {
    return res.status(400).json({ error: "sourceCode is required" });
  }

  // Sanitize program name
  const safeName = programName.toLowerCase().replace(/[^a-z0-9_-]/g, "-").slice(0, 64);
  const buildId = `${safeName}-${crypto.randomBytes(4).toString("hex")}`;

  const onBuildEvent = (event) => {
    const clients = sseClients.get(buildId);
    if (clients) {
      const data = JSON.stringify(event);
      for (const client of clients) {
        client.write(`data: ${data}\n\n`);
        if (event.type === "completed" || event.type === "error") {
          client.end();
        }
      }
      if (event.type === "completed" || event.type === "error") {
        sseClients.delete(buildId);
      }
    }
  };

  try {
    // Starts the build — validates inputs synchronously, then runs Docker in background.
    // We await only the initial setup (concurrency check, cache check, base image).
    // The actual Docker build runs as a spawned process after this resolves.
    await buildGuestProgram(buildId, safeName, sourceCode, onBuildEvent);
    res.status(202).json({ buildId, programName: safeName });
  } catch (err) {
    console.error("[guest-builds] Build start error:", err);
    const status = err.code === "CONCURRENCY_LIMIT" ? 429 : 500;
    res.status(status).json({ error: err.message || "Build failed to start" });
  }
});

/**
 * GET /api/guest-builds/:id
 * returns: { status, programName, result?, logs }
 */
router.get("/:id", (req, res) => {
  const build = getBuild(req.params.id);
  if (!build) return res.status(404).json({ error: "Build not found" });

  res.json({
    status: build.status,
    programName: build.programName,
    result: build.result,
    logCount: build.logs.length,
    logs: build.logs,
    startedAt: build.startedAt,
    sourceHash: build.sourceHash,
  });
});

/**
 * GET /api/guest-builds/:id/elf
 * returns: ELF binary file
 */
router.get("/:id/elf", (req, res) => {
  const build = getBuild(req.params.id);
  if (!build) return res.status(404).json({ error: "Build not found" });
  if (build.status !== "completed") return res.status(400).json({ error: "Build not completed" });

  const elf = getElfBuffer(req.params.id);
  if (!elf) return res.status(404).json({ error: "ELF file not found" });

  res.set("Content-Type", "application/octet-stream");
  const safeFilename = encodeURIComponent(build.programName);
  res.set("Content-Disposition", `attachment; filename="${safeFilename}.elf"`);
  res.send(elf);
});

/**
 * GET /api/guest-builds/:id/logs
 * SSE stream of build events
 */
router.get("/:id/logs", (req, res) => {
  const build = getBuild(req.params.id);
  if (!build) return res.status(404).json({ error: "Build not found" });

  // If already done, send final status and close
  if (build.status === "completed" || build.status === "error") {
    res.set("Content-Type", "text/event-stream");
    res.set("Cache-Control", "no-cache");
    res.set("Connection", "keep-alive");
    res.write(`data: ${JSON.stringify({ type: build.status, buildId: req.params.id, result: build.result })}\n\n`);
    return res.end();
  }

  // Stream events
  res.set("Content-Type", "text/event-stream");
  res.set("Cache-Control", "no-cache");
  res.set("Connection", "keep-alive");

  // Send existing logs
  for (const log of build.logs) {
    res.write(`data: ${JSON.stringify({ type: "log", buildId: req.params.id, message: log })}\n\n`);
  }

  // Register for future events
  if (!sseClients.has(req.params.id)) {
    sseClients.set(req.params.id, new Set());
  }
  sseClients.get(req.params.id).add(res);

  req.on("close", () => {
    const clients = sseClients.get(req.params.id);
    if (clients) {
      clients.delete(res);
      if (clients.size === 0) sseClients.delete(req.params.id);
    }
  });
});

/**
 * DELETE /api/guest-builds/:id
 */
router.delete("/:id", (req, res) => {
  const build = getBuild(req.params.id);
  if (!build) return res.status(404).json({ error: "Build not found" });
  if (build.status === "building") return res.status(409).json({ error: "Cannot delete a build that is in progress" });
  deleteBuild(req.params.id);
  res.json({ ok: true, deleted: req.params.id });
});

module.exports = router;

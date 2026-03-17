const express = require("express");
const router = express.Router();
const multer = require("multer");
const path = require("path");
const crypto = require("crypto");

const { requireAuth } = require("../middleware/auth");
const {
  createDeployment,
  getDeploymentsByUser,
  getDeploymentById,
  updateDeployment,
  deleteDeployment,
} = require("../db/deployments");
const { getProgramById, getProgramByProgramId, incrementUseCount } = require("../db/programs");

// Screenshot upload config
const UPLOAD_DIR = path.join(__dirname, "..", "uploads", "screenshots");
const screenshotStorage = multer.diskStorage({
  destination: (req, file, cb) => {
    const fs = require("fs");
    fs.mkdirSync(UPLOAD_DIR, { recursive: true });
    cb(null, UPLOAD_DIR);
  },
  filename: (req, file, cb) => {
    const ext = path.extname(file.originalname) || ".png";
    const hash = crypto.randomBytes(8).toString("hex");
    cb(null, `${Date.now()}-${hash}${ext}`);
  },
});
const screenshotUpload = multer({
  storage: screenshotStorage,
  limits: { fileSize: 5 * 1024 * 1024 }, // 5MB max
  fileFilter: (req, file, cb) => {
    if (file.mimetype.startsWith("image/")) cb(null, true);
    else cb(new Error("Only image files are allowed"));
  },
});

router.use(requireAuth);

// POST /api/deployments — create a new deployment record
router.post("/", (req, res) => {
  try {
    const { programId, name, chainId, rpcUrl, config } = req.body;
    if (!programId || !name) {
      return res.status(400).json({ error: "programId and name are required" });
    }
    if (typeof name !== 'string' || name.trim().length > 200) {
      return res.status(400).json({ error: "name must be a string of 200 characters or fewer" });
    }

    // Look up program by slug (program_id), not by UUID (id)
    const program = getProgramByProgramId(programId) || getProgramById(programId);
    if (!program || program.status !== "active") {
      return res.status(404).json({ error: "Program not found or not active" });
    }

    const deployment = createDeployment({
      userId: req.user.id,
      programId: program.id,  // Use UUID, not slug
      name: name.trim(),
      chainId: chainId || null,
      rpcUrl: rpcUrl || null,
      config: config || null,
    });

    incrementUseCount(program.id);
    res.status(201).json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments — list my deployments
router.get("/", (req, res) => {
  try {
    const deployments = getDeploymentsByUser(req.user.id);
    res.json({ deployments });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id — get deployment detail
router.get("/:id", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    res.json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/deployments/:id — update deployment config
router.put("/:id", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    // Prevent setting status to 'active' directly; use /activate endpoint instead
    const fields = { ...req.body };
    if (fields.status === 'active') {
      delete fields.status;
    }
    const updated = updateDeployment(req.params.id, fields);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/deployments/:id — remove deployment record
router.delete("/:id", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    deleteDeployment(req.params.id);
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/activate — mark as active (used by Desktop after successful deploy)
router.post("/:id/activate", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    const updated = updateDeployment(req.params.id, { status: "active" });
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/push-metadata — push metadata to GitHub repo
router.post("/:id/push-metadata", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    if (deployment.status !== "active") {
      return res.status(400).json({ error: "Deployment must be active to push metadata" });
    }

    const { pushMetadataToRepo } = require("../lib/metadata-push");
    const result = await pushMetadataToRepo(deployment);
    res.json({ success: true, path: result.path });
  } catch (e) {
    console.error("[push-metadata]", e.message);
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/delete-metadata — delete metadata from GitHub repo
router.post("/:id/delete-metadata", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    const { deleteMetadataFromRepo } = require("../lib/metadata-push");
    const result = await deleteMetadataFromRepo(deployment);
    res.json({ success: true, deleted: result.deleted });
  } catch (e) {
    console.error("[delete-metadata]", e.message);
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/screenshots — upload screenshot images
router.post("/:id/screenshots", screenshotUpload.array("screenshots", 10), (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!req.files || req.files.length === 0) {
      return res.status(400).json({ error: "No files uploaded" });
    }

    // Build URLs for uploaded files
    const urls = req.files.map((f) => `/uploads/screenshots/${f.filename}`);

    // Merge with existing screenshots
    let existing = [];
    try { existing = deployment.screenshots ? JSON.parse(deployment.screenshots) : []; } catch { /* ignore */ }
    const merged = [...existing, ...urls];

    // Save to DB
    updateDeployment(req.params.id, { screenshots: JSON.stringify(merged) });

    res.json({ urls, screenshots: merged });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

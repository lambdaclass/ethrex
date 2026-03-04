const express = require("express");
const router = express.Router();

const { requireAuth } = require("../middleware/auth");
const { createDeployment, getDeploymentsByUser, getDeploymentById, updateDeployment, deleteDeployment } = require("../db/deployments");
const { getProgramById, incrementUseCount } = require("../db/programs");

router.use(requireAuth);

// POST /api/deployments — create a new deployment (use a program)
router.post("/", (req, res) => {
  try {
    const { programId, name, chainId, rpcUrl, config } = req.body;
    if (!programId || !name) {
      return res.status(400).json({ error: "programId and name are required" });
    }

    const program = getProgramById(programId);
    if (!program || program.status !== "active") {
      return res.status(404).json({ error: "Program not found or not active" });
    }

    const deployment = createDeployment({
      userId: req.user.id,
      programId,
      name: name.trim(),
      chainId: chainId || null,
      rpcUrl: rpcUrl || null,
      config: config || null,
    });

    // Increment program usage count
    incrementUseCount(programId);

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
    const updated = updateDeployment(req.params.id, req.body);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/deployments/:id — remove deployment
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

// POST /api/deployments/:id/activate — change status to active
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

module.exports = router;

const express = require("express");
const router = express.Router();

const { requireAuth } = require("../middleware/auth");
const {
  createDeployment,
  getDeploymentsByUser,
  getDeploymentById,
  updateDeployment,
  deleteDeployment,
} = require("../db/deployments");
const { getProgramById, incrementUseCount } = require("../db/programs");
const {
  provision,
  provisionRemote,
  stopDeployment,
  startDeployment,
  destroyDeployment,
  getEmitter,
} = require("../lib/deployment-engine");
const docker = require("../lib/docker-local");
const remote = require("../lib/docker-remote");
const { getDeploymentDir } = require("../lib/compose-generator");
const { getHostById } = require("../db/hosts");
const rpc = require("../lib/rpc-client");
const path = require("path");
const fs = require("fs");

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
router.delete("/:id", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    // If deployment has Docker resources, destroy them first
    if (deployment.docker_project && deployment.phase !== "configured") {
      try {
        await destroyDeployment(deployment);
      } catch {
        // Continue with DB deletion even if Docker cleanup fails
      }
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

// ==========================================
// Docker Deployment Lifecycle Endpoints
// ==========================================

// POST /api/deployments/:id/provision — start full deployment
router.post("/:id/provision", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (deployment.phase === "running") {
      return res.status(400).json({ error: "Deployment is already running" });
    }

    if (["building", "l1_starting", "deploying_contracts", "l2_starting", "starting_prover"].includes(deployment.phase)) {
      return res.status(400).json({ error: "Deployment is already in progress" });
    }

    const { hostId } = req.body;

    // Start provisioning in background
    res.json({ ok: true, message: "Provisioning started", remote: !!hostId });

    // Run async — progress is tracked via SSE /events endpoint
    const provisionFn = hostId
      ? () => provisionRemote(deployment, hostId)
      : () => provision(deployment);

    provisionFn().catch((err) => {
      console.error(`Provision failed for ${deployment.id}:`, err.message);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/start — restart stopped deployment
router.post("/:id/start", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!deployment.docker_project) {
      return res.status(400).json({ error: "Deployment has not been provisioned yet" });
    }

    const updated = await startDeployment(deployment);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/stop — stop deployment (keep volumes)
router.post("/:id/stop", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!deployment.docker_project) {
      return res.status(400).json({ error: "Deployment has not been provisioned yet" });
    }

    const updated = await stopDeployment(deployment);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/destroy — destroy deployment completely
router.post("/:id/destroy", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!deployment.docker_project) {
      return res.status(400).json({ error: "Deployment has not been provisioned yet" });
    }

    const updated = await destroyDeployment(deployment);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/status — container status + endpoints
router.get("/:id/status", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!deployment.docker_project) {
      return res.json({
        phase: deployment.phase,
        containers: [],
        endpoints: {},
      });
    }

    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    let containers = [];
    if (fs.existsSync(composeFile)) {
      containers = await docker.getStatus(deployment.docker_project, composeFile);
    }

    res.json({
      phase: deployment.phase,
      containers,
      endpoints: {
        l1Rpc: deployment.l1_port ? `http://127.0.0.1:${deployment.l1_port}` : null,
        l2Rpc: deployment.l2_port ? `http://127.0.0.1:${deployment.l2_port}` : null,
      },
      contracts: {
        bridge: deployment.bridge_address,
        proposer: deployment.proposer_address,
      },
      error: deployment.error_message,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/events — SSE stream of deployment progress
router.get("/:id/events", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    // SSE headers
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });

    // Send current phase immediately
    res.write(`data: ${JSON.stringify({ event: "phase", phase: deployment.phase, timestamp: Date.now() })}\n\n`);

    const emitter = getEmitter(deployment.id);
    const handler = (data) => {
      res.write(`data: ${JSON.stringify(data)}\n\n`);

      // Close stream when deployment reaches terminal state
      if (data.phase === "running" || data.event === "error") {
        setTimeout(() => res.end(), 1000);
      }
    };

    emitter.on("event", handler);

    req.on("close", () => {
      emitter.removeListener("event", handler);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/logs — service logs (SSE or plain)
router.get("/:id/logs", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!deployment.docker_project) {
      return res.status(400).json({ error: "Deployment has not been provisioned yet" });
    }

    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    if (!fs.existsSync(composeFile)) {
      return res.status(400).json({ error: "Compose file not found" });
    }

    const service = req.query.service || null;
    const follow = req.query.follow === "true";
    const tail = parseInt(req.query.tail) || 100;

    if (follow) {
      // SSE streaming logs
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      });

      const proc = docker.streamLogs(deployment.docker_project, composeFile, service);

      proc.stdout.on("data", (chunk) => {
        const lines = chunk.toString().split("\n").filter(Boolean);
        for (const line of lines) {
          res.write(`data: ${JSON.stringify({ line })}\n\n`);
        }
      });

      proc.stderr.on("data", (chunk) => {
        const lines = chunk.toString().split("\n").filter(Boolean);
        for (const line of lines) {
          res.write(`data: ${JSON.stringify({ line })}\n\n`);
        }
      });

      proc.on("close", () => res.end());
      req.on("close", () => proc.kill("SIGTERM"));
    } else {
      // Return logs as plain text
      const logs = await docker.getLogs(deployment.docker_project, composeFile, service, tail);
      res.json({ logs });
    }
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/monitoring — block height, chain info, balances
router.get("/:id/monitoring", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (!deployment.l1_port || !deployment.l2_port) {
      return res.json({ l1: null, l2: null });
    }

    // For remote deployments, use the host's IP; for local, use 127.0.0.1
    let rpcHost = "127.0.0.1";
    if (deployment.host_id) {
      const host = getHostById(deployment.host_id);
      if (host) rpcHost = host.hostname;
    }
    const l1Url = `http://${rpcHost}:${deployment.l1_port}`;
    const l2Url = `http://${rpcHost}:${deployment.l2_port}`;

    // Prefunded account (from private_keys_l1.txt first key)
    const prefundedAddress = "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b";

    const [l1Block, l2Block, l1Chain, l2Chain, l1Balance, l2Balance] = await Promise.allSettled([
      rpc.getBlockNumber(l1Url),
      rpc.getBlockNumber(l2Url),
      rpc.getChainId(l1Url),
      rpc.getChainId(l2Url),
      rpc.getBalance(l1Url, prefundedAddress),
      rpc.getBalance(l2Url, prefundedAddress),
    ]);

    res.json({
      l1: {
        healthy: l1Block.status === "fulfilled",
        blockNumber: l1Block.status === "fulfilled" ? l1Block.value : null,
        chainId: l1Chain.status === "fulfilled" ? l1Chain.value : null,
        balance: l1Balance.status === "fulfilled" ? l1Balance.value : null,
        rpcUrl: l1Url,
      },
      l2: {
        healthy: l2Block.status === "fulfilled",
        blockNumber: l2Block.status === "fulfilled" ? l2Block.value : null,
        chainId: l2Chain.status === "fulfilled" ? l2Chain.value : null,
        balance: l2Balance.status === "fulfilled" ? l2Balance.value : null,
        rpcUrl: l2Url,
      },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

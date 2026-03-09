const express = require("express");
const router = express.Router();
const { v4: uuidv4 } = require("uuid");

const {
  provision,
  provisionRemote,
  stopDeployment,
  startDeployment,
  destroyDeployment,
  getEmitter,
  isProvisionActive,
  cancelProvision,
  getActiveProvisions,
} = require("../lib/deployment-engine");
const { getDeployEvents } = require("../db/deployments");
const docker = require("../lib/docker-local");
const remote = require("../lib/docker-remote");
const { getDeploymentDir } = require("../lib/compose-generator");
const rpc = require("../lib/rpc-client");
const db = require("../db/db");
const path = require("path");
const fs = require("fs");

// ==========================================
// CRUD (local — no auth required)
// ==========================================

// POST /api/deployments — create a new deployment
router.post("/", (req, res) => {
  try {
    const { programSlug, name, chainId, config } = req.body;
    if (!name) {
      return res.status(400).json({ error: "name is required" });
    }

    const id = uuidv4();
    const now = Date.now();

    db.prepare(`
      INSERT INTO deployments (id, program_slug, name, chain_id, config, created_at)
      VALUES (?, ?, ?, ?, ?, ?)
    `).run(id, programSlug || "evm-l2", name.trim(), chainId || null, config ? JSON.stringify(config) : null, now);

    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(id);
    res.status(201).json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments — list all local deployments
router.get("/", (req, res) => {
  try {
    const deployments = db.prepare("SELECT * FROM deployments ORDER BY created_at DESC").all();
    res.json({ deployments });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/docker/status — check if Docker daemon is available
router.get("/docker/status", (req, res) => {
  try {
    const available = docker.isDockerAvailable();
    res.json({ available });
  } catch (e) {
    res.json({ available: false });
  }
});

// GET /api/deployments/:id — get deployment detail
router.get("/:id", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    res.json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/deployments/:id — update deployment
router.put("/:id", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    const allowedFields = ["name", "chain_id", "rpc_url", "is_public"];
    const updates = [];
    const values = [];

    for (const field of allowedFields) {
      if (req.body[field] !== undefined) {
        updates.push(`${field} = ?`);
        values.push(req.body[field]);
      }
    }

    if (updates.length > 0) {
      values.push(req.params.id);
      db.prepare(`UPDATE deployments SET ${updates.join(", ")} WHERE id = ?`).run(...values);
    }

    const updated = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/deployments/:id — remove deployment
router.delete("/:id", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    // Cancel active provision if running
    cancelProvision(req.params.id);

    // Cleanup Docker resources if any
    if (deployment.docker_project && deployment.phase !== "configured") {
      try {
        await destroyDeployment(deployment);
      } catch {
        // Continue with DB deletion
      }
    }

    db.prepare("DELETE FROM deploy_events WHERE deployment_id = ?").run(req.params.id);
    db.prepare("DELETE FROM deployments WHERE id = ?").run(req.params.id);
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ==========================================
// Docker Deployment Lifecycle
// ==========================================

// Helper: update deployment in DB
function updateDeployment(id, fields) {
  const updates = [];
  const values = [];
  for (const [key, val] of Object.entries(fields)) {
    updates.push(`${key} = ?`);
    values.push(val);
  }
  if (updates.length > 0) {
    values.push(id);
    db.prepare(`UPDATE deployments SET ${updates.join(", ")} WHERE id = ?`).run(...values);
  }
  return db.prepare("SELECT * FROM deployments WHERE id = ?").get(id);
}

// POST /api/deployments/:id/provision — start full deployment
router.post("/:id/provision", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (deployment.phase === "running") {
      return res.status(400).json({ error: "Deployment is already running" });
    }

    const inProgressPhases = ["checking_docker", "building", "l1_starting", "deploying_contracts", "l2_starting", "starting_prover", "starting_tools"];
    if (inProgressPhases.includes(deployment.phase)) {
      return res.status(400).json({ error: "Deployment is already in progress" });
    }

    const { hostId } = req.body;
    res.json({ ok: true, message: "Provisioning started", remote: !!hostId });

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

// POST /api/deployments/:id/start
router.post("/:id/start", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    const updated = await startDeployment(deployment);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/stop
router.post("/:id/stop", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    const updated = await stopDeployment(deployment);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/destroy
router.post("/:id/destroy", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    // Cancel active provision if running
    cancelProvision(req.params.id);

    // Destroy Docker containers if provisioned
    if (deployment.docker_project) {
      await destroyDeployment(deployment);
    }

    // Remove from DB
    db.prepare("DELETE FROM deploy_events WHERE deployment_id = ?").run(req.params.id);
    db.prepare("DELETE FROM deployments WHERE id = ?").run(req.params.id);
    res.json({ ok: true, deleted: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Tools services live in a separate compose file
const TOOLS_SERVICES = new Set(["frontend-l1", "backend-l1", "frontend-l2", "backend-l2", "db", "db-init", "redis-db", "proxy", "function-selectors", "bridge-ui"]);

// POST /api/deployments/:id/service/:service/stop — stop a single service
router.post("/:id/service/:service/stop", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });
    if (TOOLS_SERVICES.has(req.params.service)) {
      // Tools use separate compose — stop via tools compose
      await docker.stopTools();
      return res.json({ ok: true, message: `Tools stopped` });
    }
    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    await docker.stopService(deployment.docker_project, composeFile, req.params.service);
    res.json({ ok: true, message: `Service ${req.params.service} stopped` });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/service/:service/start — start a single service
router.post("/:id/service/:service/start", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });
    if (TOOLS_SERVICES.has(req.params.service)) {
      // Tools use separate compose — start all tools together (they depend on each other)
      const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
      const envVars = await docker.extractEnv(deployment.docker_project, composeFile);
      await docker.startTools(envVars, {
        toolsL1ExplorerPort: deployment.tools_l1_explorer_port,
        toolsL2ExplorerPort: deployment.tools_l2_explorer_port,
        toolsBridgeUIPort: deployment.tools_bridge_ui_port,
        toolsDbPort: deployment.tools_db_port,
        l1Port: deployment.l1_port,
        l2Port: deployment.l2_port,
        toolsMetricsPort: deployment.tools_metrics_port,
      });
      return res.json({ ok: true, message: `Tools started` });
    }
    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    await docker.startService(deployment.docker_project, composeFile, req.params.service, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
    res.json({ ok: true, message: `Service ${req.params.service} started` });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/build-tools
router.post("/:id/build-tools", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    const toolsPorts = {
      toolsL1ExplorerPort: deployment.tools_l1_explorer_port,
      toolsL2ExplorerPort: deployment.tools_l2_explorer_port,
      toolsBridgeUIPort: deployment.tools_bridge_ui_port,
      toolsDbPort: deployment.tools_db_port,
      toolsMetricsPort: deployment.tools_metrics_port,
      l1Port: deployment.l1_port,
      l2Port: deployment.l2_port,
    };

    await docker.buildTools(toolsPorts);
    res.json({ ok: true, message: "Tools images rebuilt" });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/restart-tools
router.post("/:id/restart-tools", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    let envVars = {};
    try {
      const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
      const volumeEnv = await docker.extractEnv(deployment.docker_project, composeFile);
      if (volumeEnv.ETHREX_WATCHER_BRIDGE_ADDRESS) envVars.ETHREX_WATCHER_BRIDGE_ADDRESS = volumeEnv.ETHREX_WATCHER_BRIDGE_ADDRESS;
      if (volumeEnv.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS) envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS = volumeEnv.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS;
    } catch {
      if (deployment.bridge_address) envVars.ETHREX_WATCHER_BRIDGE_ADDRESS = deployment.bridge_address;
      if (deployment.proposer_address) envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS = deployment.proposer_address;
    }

    const toolsPorts = {
      toolsL1ExplorerPort: deployment.tools_l1_explorer_port,
      toolsL2ExplorerPort: deployment.tools_l2_explorer_port,
      toolsBridgeUIPort: deployment.tools_bridge_ui_port,
      toolsDbPort: deployment.tools_db_port,
      toolsMetricsPort: deployment.tools_metrics_port,
      l1Port: deployment.l1_port,
      l2Port: deployment.l2_port,
    };

    await docker.restartTools(envVars, toolsPorts);
    res.json({ ok: true, message: "Tools restarted" });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/stop-tools
router.post("/:id/stop-tools", async (req, res) => {
  try {
    await docker.stopTools();
    res.json({ ok: true, message: "Tools stopped" });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ==========================================
// Monitoring & Logs
// ==========================================

// GET /api/deployments/:id/status
router.get("/:id/status", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    if (!deployment.docker_project) {
      return res.json({ phase: deployment.phase, containers: [], endpoints: {} });
    }

    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    let containers = [];
    if (fs.existsSync(composeFile)) {
      containers = await docker.getStatus(deployment.docker_project, composeFile);
    }
    // Also fetch tools containers (Explorer, Bridge UI, etc.)
    try {
      const toolsContainers = await docker.getToolsStatus();
      if (toolsContainers.length > 0) {
        containers = containers.concat(toolsContainers);
      }
    } catch {}


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

// GET /api/deployments/:id/events — SSE stream
router.get("/:id/events", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });

    res.write(`data: ${JSON.stringify({ event: "phase", phase: deployment.phase, timestamp: Date.now() })}\n\n`);

    const emitter = getEmitter(deployment.id);
    const handler = (data) => {
      res.write(`data: ${JSON.stringify(data)}\n\n`);
      if (data.phase === "running" || data.event === "error") {
        setTimeout(() => res.end(), 1000);
      }
    };

    emitter.on("event", handler);
    req.on("close", () => emitter.removeListener("event", handler));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/events/history — get stored events from DB
router.get("/:id/events/history", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    const since = req.query.since ? parseInt(req.query.since) : undefined;
    const limit = req.query.limit ? parseInt(req.query.limit) : 1000;
    const events = getDeployEvents(deployment.id, { since, limit });

    res.json({
      events,
      isActive: isProvisionActive(deployment.id),
      phase: deployment.phase,
      createdAt: deployment.created_at,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/active/provisions — list currently running provisions
router.get("/active/provisions", (req, res) => {
  res.json({ provisions: getActiveProvisions() });
});

// GET /api/deployments/:id/logs
router.get("/:id/logs", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    if (!fs.existsSync(composeFile)) {
      return res.status(400).json({ error: "Compose file not found" });
    }

    const service = req.query.service || null;
    const follow = req.query.follow === "true";
    const tail = parseInt(req.query.tail) || 100;

    const toolsServices = ["bridge-ui", "db", "backend-l1", "backend-l2", "frontend-l1", "frontend-l2", "proxy"];
    const isToolsService = service && toolsServices.includes(service);

    if (follow) {
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      });

      const proc = isToolsService
        ? docker.streamToolsLogs(service)
        : docker.streamLogs(deployment.docker_project, composeFile, service);

      proc.stdout.on("data", (chunk) => {
        for (const line of chunk.toString().split("\n").filter(Boolean)) {
          res.write(`data: ${JSON.stringify({ line })}\n\n`);
        }
      });
      proc.stderr.on("data", (chunk) => {
        for (const line of chunk.toString().split("\n").filter(Boolean)) {
          res.write(`data: ${JSON.stringify({ line })}\n\n`);
        }
      });
      proc.on("close", () => res.end());
      req.on("close", () => proc.kill("SIGTERM"));
    } else {
      const logs = isToolsService
        ? await docker.getToolsLogs(service, tail)
        : await docker.getLogs(deployment.docker_project, composeFile, service, tail);
      res.json({ logs });
    }
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/monitoring
router.get("/:id/monitoring", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    if (!deployment.l1_port || !deployment.l2_port) {
      return res.json({ l1: null, l2: null });
    }

    let rpcHost = "127.0.0.1";
    if (deployment.host_id) {
      const host = db.prepare("SELECT * FROM hosts WHERE id = ?").get(deployment.host_id);
      if (host) rpcHost = host.hostname;
    }

    const l1Url = `http://${rpcHost}:${deployment.l1_port}`;
    const l2Url = `http://${rpcHost}:${deployment.l2_port}`;
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

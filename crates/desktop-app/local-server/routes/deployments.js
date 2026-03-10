const express = require("express");
const router = express.Router();
const { v4: uuidv4 } = require("uuid");
const { ethers } = require("ethers");

const {
  provision,
  provisionTestnet,
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
const keychain = require("../lib/keychain");
const { getExternalL1Config } = require("../lib/tools-config");
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

// Estimated gas per role:
// Deployer: ~5 contracts × 2 txs (impl + proxy) + timelock init = ~25M gas (one-time)
// Committer: batch commit ~200K gas per tx (recurring, estimate first month ~100 txs)
// Proof Coordinator: proof submission ~300K gas per tx (recurring, estimate first month ~100 txs)
// Bridge Owner: VK registration + accept ownership ~500K gas (one-time setup)
const ROLE_GAS_ESTIMATES = {
  deployer: {
    gas: 25_000_000,
    label: "Contract deployment (one-time)",
    detail: "5 contracts (Bridge, Proposer, Timelock, SP1Verifier, SequencerRegistry) × 2 txs (impl + proxy) ≈ 2.5M gas/pair × 5 = ~25M gas",
  },
  committer: {
    gas: 8_640_000_000,
    label: "Batch commits (1 month, 60s interval)",
    detail: "commit_batch() ≈ 200K gas/tx. Commits every 60s including empty blocks. 1,440/day × 30 days = 43,200 tx/month. 200K × 43,200 = ~8.64B gas/month.",
    interval: "60s",
  },
  "proof-coordinator": {
    gas: 12_960_000_000,
    label: "Proof submissions (1 month, 1:1 with commits)",
    detail: "verify() ≈ 300K gas/tx (ZK proof on-chain). 1 proof per committed batch. 43,200 tx/month. 300K × 43,200 = ~12.96B gas/month.",
    interval: "60s (1:1 with commits)",
  },
  "bridge-owner": {
    gas: 500_000,
    label: "VK registration + ownership (one-time)",
    detail: "setVerificationKey() ≈ 200K gas + acceptOwnership() × 2 contracts ≈ 100K each + register() ≈ 100K = ~500K gas total",
  },
};

/** Validate RPC URL: must be http(s), no private IPs or metadata endpoints.
 *  allowLocal=true permits localhost/127.0.0.1 (for local L1 dev mode). */
function validateRpcUrl(rpcUrl, { allowLocal = false } = {}) {
  let parsed;
  try { parsed = new URL(rpcUrl); } catch { throw new Error("Invalid URL format"); }
  if (!["http:", "https:"].includes(parsed.protocol)) throw new Error("URL must use http or https");
  const host = parsed.hostname;
  // Block cloud metadata endpoints
  if (host === "169.254.169.254" || host === "metadata.google.internal") throw new Error("Blocked: cloud metadata endpoint");
  // Block private IPs (RFC1918)
  if (/^(10\.|172\.(1[6-9]|2\d|3[01])\.|192\.168\.)/.test(host)) throw new Error("Blocked: private IP range");
  // Block loopback unless explicitly allowed (local L1 dev mode)
  if (!allowLocal && (host === "127.0.0.1" || host === "::1" || host === "0.0.0.0" || host === "localhost")) {
    throw new Error("Blocked: localhost/loopback address. Use an external RPC URL for testnet.");
  }
  return parsed;
}

// Shared RPC call helper with 10s timeout
function makeRpcCaller(rpcUrl) {
  validateRpcUrl(rpcUrl);
  return async (method, params = []) => {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 10000);
    try {
      const r = await globalThis.fetch(rpcUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
        signal: controller.signal,
      });
      const data = await r.json();
      if (data.error) throw new Error(data.error.message || "RPC error");
      return data.result;
    } finally {
      clearTimeout(timeout);
    }
  };
}

const CHAIN_NAMES = { 1: "Ethereum Mainnet", 11155111: "Sepolia", 17000: "Holesky" };

function formatGwei(gasPriceGwei) {
  return gasPriceGwei < 0.0001 ? gasPriceGwei.toPrecision(4) : gasPriceGwei.toFixed(4);
}

/** Convert wei BigInt to ETH string with 6 decimal precision (safe for large values) */
function weiToEth(wei) {
  const ETH = 1000000000000000000n; // 1e18
  const whole = wei / ETH;
  const remainder = wei % ETH;
  const decimal = remainder * 1000000n / ETH; // 6 decimal places
  return `${whole}.${decimal.toString().padStart(6, "0")}`;
}

// POST /api/deployments/testnet/check-balance — check account balance on testnet
router.post("/testnet/check-balance", async (req, res) => {
  try {
    const { rpcUrl, address, role } = req.body;
    if (!rpcUrl || !address) {
      return res.status(400).json({ error: "rpcUrl and address are required" });
    }
    if (!/^0x[0-9a-fA-F]{40}$/.test(address)) {
      return res.status(400).json({ error: "Invalid Ethereum address format" });
    }

    const rpcCall = makeRpcCaller(rpcUrl);
    const [balanceHex, chainIdHex, gasPriceHex] = await Promise.all([
      rpcCall("eth_getBalance", [address, "latest"]),
      rpcCall("eth_chainId"),
      rpcCall("eth_gasPrice"),
    ]);

    const balanceWei = BigInt(balanceHex || "0x0");
    const chainId = parseInt(chainIdHex || "0x0", 16);
    const gasPriceWei = BigInt(gasPriceHex || "0x0");
    const gasPriceGwei = Number(gasPriceWei / 1000n) / 1e6;

    const roleInfo = ROLE_GAS_ESTIMATES[role] || ROLE_GAS_ESTIMATES.deployer;
    const estimatedGas = roleInfo.gas;
    const estimatedCostWei = gasPriceWei * BigInt(estimatedGas);
    const sufficient = balanceWei >= estimatedCostWei;

    res.json({
      address,
      role: role || "deployer",
      balanceEth: weiToEth(balanceWei),
      chainId,
      gasPriceGwei: formatGwei(gasPriceGwei),
      estimatedGas,
      gasLabel: roleInfo.label,
      gasDetail: roleInfo.detail,
      interval: roleInfo.interval || null,
      estimatedCostEth: weiToEth(estimatedCostWei),
      sufficient,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/testnet/check-rpc — test L1 RPC connectivity
router.post("/testnet/check-rpc", async (req, res) => {
  try {
    const { rpcUrl } = req.body;
    if (!rpcUrl) {
      return res.status(400).json({ error: "rpcUrl is required" });
    }

    const rpcCall = makeRpcCaller(rpcUrl);
    const [chainIdHex, blockHex] = await Promise.all([
      rpcCall("eth_chainId"),
      rpcCall("eth_blockNumber"),
    ]);

    const chainId = parseInt(chainIdHex || "0x0", 16);
    const blockNumber = parseInt(blockHex || "0x0", 16);
    const chainName = CHAIN_NAMES[chainId] || `Chain ${chainId}`;

    res.json({ ok: true, chainId, chainName, blockNumber });
  } catch (e) {
    res.status(500).json({ ok: false, error: e.message });
  }
});

// GET /api/deployments/keychain/accounts — list keychain accounts
router.get("/keychain/accounts", (req, res) => {
  try {
    const accounts = keychain.listAccounts();
    res.json({ accounts });
  } catch (e) {
    res.json({ accounts: [] });
  }
});

// POST /api/deployments/testnet/resolve-keys — resolve keychain keys to addresses + balances
router.post("/testnet/resolve-keys", async (req, res) => {
  try {
    const { rpcUrl, deployerKey, committerKey, proofCoordinatorKey, bridgeOwnerKey } = req.body;
    if (!rpcUrl) {
      return res.status(400).json({ error: "rpcUrl is required" });
    }

    const resolveRole = (keychainName, label) => {
      if (!keychainName) return null;
      let pk = keychain.getSecret(keychainName);
      if (!pk) return { error: `Key "${keychainName}" not found in Keychain`, label };
      try {
        const wallet = new ethers.Wallet(pk);
        const address = wallet.address;
        return { address, label, keychainName };
      } catch {
        return { error: `Invalid key format for "${keychainName}"`, label };
      } finally {
        pk = null; // Best-effort: dereference private key (actual GC timing is V8-controlled)
      }
    };

    const roles = {
      deployer: resolveRole(deployerKey, "Deployer"),
      committer: resolveRole(committerKey, "Committer") || resolveRole(deployerKey, "Committer"),
      proofCoordinator: resolveRole(proofCoordinatorKey, "Proof Coordinator") || resolveRole(deployerKey, "Proof Coordinator"),
      bridgeOwner: resolveRole(bridgeOwnerKey, "Bridge Owner") || resolveRole(deployerKey, "Bridge Owner"),
    };

    const errors = Object.values(roles).filter(r => r?.error);
    if (errors.length > 0) {
      return res.status(400).json({ error: errors[0].error });
    }

    // Fetch balances in parallel using BigInt for precision
    const rpcCall = makeRpcCaller(rpcUrl);
    const uniqueAddresses = [...new Set(Object.values(roles).map(r => r?.address).filter(Boolean))];
    const balanceWeis = {};
    const [gasPriceHex] = await Promise.all([
      rpcCall("eth_gasPrice"),
      ...uniqueAddresses.map(async (addr) => {
        const hex = await rpcCall("eth_getBalance", [addr, "latest"]);
        balanceWeis[addr] = BigInt(hex || "0x0");
      }),
    ]);

    const gasPriceWei = BigInt(gasPriceHex || "0x0");
    const gasPriceGwei = Number(gasPriceWei / 1000n) / 1e6;

    // Estimate total deployment cost using BigInt
    const deployerGas = ROLE_GAS_ESTIMATES.deployer.gas;
    const estimatedCostWei = gasPriceWei * BigInt(deployerGas);

    // Enrich roles with balances
    for (const role of Object.values(roles)) {
      if (role?.address) {
        const wei = balanceWeis[role.address] || 0n;
        role.balance = weiToEth(wei);
      }
    }

    const deployerWei = roles.deployer?.address ? (balanceWeis[roles.deployer.address] || 0n) : 0n;

    res.json({
      roles,
      gasPriceGwei: formatGwei(gasPriceGwei),
      estimatedDeployCostEth: weiToEth(estimatedCostWei),
      deployerSufficient: deployerWei >= estimatedCostWei,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/testnet/estimate-gas — estimate total deployment + operational costs
router.post("/testnet/estimate-gas", async (req, res) => {
  try {
    const { rpcUrl } = req.body;
    if (!rpcUrl) {
      return res.status(400).json({ error: "rpcUrl is required" });
    }

    const rpcCall = makeRpcCaller(rpcUrl);
    const [gasPriceHex, chainIdHex] = await Promise.all([
      rpcCall("eth_gasPrice"),
      rpcCall("eth_chainId"),
    ]);

    const gasPriceWei = BigInt(gasPriceHex || "0x0");
    const gasPriceGwei = Number(gasPriceWei / 1000n) / 1e6;
    const chainId = parseInt(chainIdHex || "0x0", 16);

    const breakdown = {};
    let totalGas = 0n;
    for (const [role, info] of Object.entries(ROLE_GAS_ESTIMATES)) {
      const gasBI = BigInt(info.gas);
      const costWei = gasPriceWei * gasBI;
      totalGas += gasBI;
      breakdown[role] = {
        gas: info.gas,
        label: info.label,
        detail: info.detail,
        interval: info.interval || null,
        costEth: weiToEth(costWei),
      };
    }

    const totalCostWei = gasPriceWei * totalGas;

    res.json({
      chainId,
      chainName: CHAIN_NAMES[chainId] || `Chain ${chainId}`,
      gasPriceGwei: formatGwei(gasPriceGwei),
      breakdown,
      totalGas: totalGas.toString(),
      totalCostEth: weiToEth(totalCostWei),
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/active/provisions — list currently running provisions
// NOTE: Must be defined before /:id to avoid route shadowing
router.get("/active/provisions", (req, res) => {
  res.json({ provisions: getActiveProvisions() });
});

// GET /api/deployments/check-image/:slug — check if Docker image exists for a program
router.get("/check-image/:slug", (req, res) => {
  try {
    const image = docker.findImage(req.params.slug);
    res.json({ exists: !!image, image: image || null });
  } catch (e) {
    res.json({ exists: false, image: null });
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

    const allowedFields = ["name", "chain_id", "rpc_url", "config", "is_public", "hashtags"];
    const updates = [];
    const values = [];

    for (const field of allowedFields) {
      if (req.body[field] !== undefined) {
        updates.push(`${field} = ?`);
        values.push(field === 'config' && typeof req.body[field] === 'object' ? JSON.stringify(req.body[field]) : req.body[field]);
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
    // Determine mode from deployment config
    let deployMode = 'local';
    try {
      const config = deployment.config ? JSON.parse(deployment.config) : {};
      deployMode = config.mode || 'local';
    } catch {}

    res.json({ ok: true, message: "Provisioning started", remote: !!hostId, mode: deployMode });

    const provisionFn = hostId
      ? () => provisionRemote(deployment, hostId)
      : (deployMode === 'testnet' ? () => provisionTestnet(deployment) : () => provision(deployment));

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

    // Cancel active provision first (removes from registry)
    const wasProvisioning = cancelProvision(req.params.id);

    if (deployment.docker_project) {
      // Stop all containers including deployer
      try {
        const path = require("path");
        const { getDeploymentDir } = require("../lib/compose-generator");
        const composeFile = path.join(getDeploymentDir(deployment.id), "docker-compose.yaml");
        const docker = require("../lib/docker-local");
        // Stop tools first, then core services
        try { await docker.stopTools(); } catch { /* tools may not be running */ }
        await docker.stop(deployment.docker_project, composeFile);
      } catch (e) {
        console.log(`[stop] docker stop failed: ${e.message}`);
      }
      const { updateDeployment: updateDep } = require("../db/deployments");
      const updated = updateDep(deployment.id, {
        phase: "stopped",
        error_message: wasProvisioning ? "Cancelled by user" : null,
      });
      res.json({ deployment: updated });
    } else {
      // No docker project yet — just mark as configured
      const { updateDeployment: updateDep } = require("../db/deployments");
      const updated = updateDep(deployment.id, {
        phase: "configured",
        error_message: wasProvisioning ? "Cancelled by user" : null,
      });
      res.json({ deployment: updated });
    }
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
const TOOLS_SERVICES = new Set(["frontend-l1", "backend-l1", "frontend-l2", "backend-l2", "db", "db-init", "redis-db", "proxy", "proxy-l2-only", "function-selectors", "function-selectors-l2", "bridge-ui"]);

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
        ...getExternalL1Config(deployment),
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
      ...getExternalL1Config(deployment),
    };

    // Respond immediately — docker compose up can take 30s+ and WebKit times out
    res.json({ ok: true, message: "Tools starting..." });
    docker.restartTools(envVars, toolsPorts).catch(e => {
      console.error("Tools restart failed:", e.message);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/stop-tools
router.post("/:id/stop-tools", async (req, res) => {
  try {
    res.json({ ok: true, message: "Tools stopping..." });
    docker.stopTools().catch(e => {
      console.error("Tools stop failed:", e.message);
    });
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

// (moved above /:id to avoid route shadowing)

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

    if (!deployment.l2_port) {
      return res.json({ l1: null, l2: null });
    }

    let rpcHost = "127.0.0.1";
    if (deployment.host_id) {
      const host = db.prepare("SELECT * FROM hosts WHERE id = ?").get(deployment.host_id);
      if (host) rpcHost = host.hostname;
    }

    // Testnet: no local L1 port, use external rpc_url instead
    const l1Url = deployment.l1_port
      ? `http://${rpcHost}:${deployment.l1_port}`
      : deployment.rpc_url || null;
    const l2Url = `http://${rpcHost}:${deployment.l2_port}`;
    const prefundedAddress = "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b";

    const reject = Promise.reject(new Error("no url"));
    reject.catch(() => {}); // suppress unhandled rejection
    const [l1Block, l2Block, l1Chain, l2Chain, l1Balance, l2Balance] = await Promise.allSettled([
      l1Url ? rpc.getBlockNumber(l1Url) : reject,
      rpc.getBlockNumber(l2Url),
      l1Url ? rpc.getChainId(l1Url) : reject,
      rpc.getChainId(l2Url),
      l1Url ? rpc.getBalance(l1Url, prefundedAddress) : reject,
      rpc.getBalance(l2Url, prefundedAddress),
    ]);

    res.json({
      l1: l1Url ? {
        healthy: l1Block.status === "fulfilled",
        blockNumber: l1Block.status === "fulfilled" ? l1Block.value : null,
        chainId: l1Chain.status === "fulfilled" ? l1Chain.value : null,
        balance: l1Balance.status === "fulfilled" ? l1Balance.value : null,
        rpcUrl: l1Url,
      } : null,
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

// (moved above /:id to avoid route shadowing)

module.exports = router;

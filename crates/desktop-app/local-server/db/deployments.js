const { v4: uuidv4 } = require("uuid");
const net = require("net");
const db = require("./db");

function createDeployment({ programSlug, name, chainId, rpcUrl, config, stackType }) {
  const id = uuidv4();
  const now = Date.now();
  db.prepare(
    `INSERT INTO deployments (id, program_slug, stack_type, name, chain_id, rpc_url, config, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)`
  ).run(id, programSlug || "evm-l2", stackType || "ethrex", name, chainId || null, rpcUrl || null, config ? JSON.stringify(config) : null, now);
  return getDeploymentById(id);
}

function getDeploymentById(id) {
  return db.prepare("SELECT * FROM deployments WHERE id = ?").get(id);
}

function getAllDeployments() {
  return db.prepare("SELECT * FROM deployments ORDER BY created_at DESC").all();
}

function updateDeployment(id, fields) {
  const allowed = [
    "name", "stack_type", "chain_id", "l1_chain_id", "rpc_url", "status", "config",
    "docker_project", "deploy_dir",
    "l1_port", "l2_port", "proof_coord_port",
    "phase", "bridge_address", "proposer_address", "timelock_address", "sp1_verifier_address",
    "guest_program_registry_address", "verification_status", "error_message",
    "host_id", "is_public", "public_domain",
    "public_l2_rpc_url", "public_l2_explorer_url", "public_l1_explorer_url", "public_dashboard_url",
    "tools_l1_explorer_port", "tools_l2_explorer_port",
    "tools_bridge_ui_port", "tools_db_port", "tools_metrics_port",
    "env_project_id", "env_updated_at", "ever_running",
    "platform_deployment_id",
  ];
  const updates = [];
  const values = [];
  for (const [key, value] of Object.entries(fields)) {
    if (allowed.includes(key)) {
      updates.push(`${key} = ?`);
      values.push(key === "config" && typeof value === "object" ? JSON.stringify(value) : value);
    }
  }
  if (updates.length === 0) return getDeploymentById(id);
  values.push(id);
  db.prepare(`UPDATE deployments SET ${updates.join(", ")} WHERE id = ?`).run(...values);
  return getDeploymentById(id);
}

function deleteDeployment(id) {
  db.prepare("DELETE FROM deployments WHERE id = ?").run(id);
}

/**
 * Check if a TCP port is actually available on 127.0.0.1.
 * Returns a promise that resolves to true if the port is free.
 */
function isPortFree(port) {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.once("error", () => resolve(false));
    server.once("listening", () => {
      server.close(() => resolve(true));
    });
    server.listen(port, "127.0.0.1");
  });
}

/**
 * Find the next free port starting from `start`, checking actual TCP availability.
 */
async function findFreePort(start, maxAttempts = 100) {
  for (let port = start; port < start + maxAttempts; port++) {
    if (await isPortFree(port)) return port;
  }
  throw new Error(`No free port found starting from ${start}`);
}

async function getNextAvailablePorts() {
  const result = db.prepare(
    `SELECT MAX(l1_port) as max_l1, MAX(l2_port) as max_l2, MAX(proof_coord_port) as max_pc,
            MAX(tools_l1_explorer_port) as max_tl1, MAX(tools_l2_explorer_port) as max_tl2,
            MAX(tools_bridge_ui_port) as max_tbridge, MAX(tools_db_port) as max_tdb,
            MAX(tools_metrics_port) as max_tmetrics
     FROM deployments`
  ).get();

  // Non-overlapping port groups can be allocated in parallel
  const [l1Port, l2Port, proofCoordPort, toolsBridgeUIPort, toolsDbPort, toolsMetricsPort] =
    await Promise.all([
      findFreePort((result.max_l1 || 8544) + 1),
      findFreePort((result.max_l2 || 1728) + 1),
      findFreePort((result.max_pc || 3899) + 1),
      findFreePort((result.max_tbridge || 3009) + 1),
      findFreePort((result.max_tdb || 7432) + 1),
      findFreePort((result.max_tmetrics || 3701) + 1),
    ]);

  // Explorer ports share the 808x range — allocate sequentially to avoid collisions
  const maxExplorer = Math.max(result.max_tl1 || 8083, result.max_tl2 || 8082);
  const toolsL2ExplorerPort = await findFreePort(maxExplorer + 1);
  const toolsL1ExplorerPort = await findFreePort(toolsL2ExplorerPort + 1);

  return { l1Port, l2Port, proofCoordPort, toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, toolsMetricsPort };
}

// ============================================================
// Deploy Events (persistent log)
// ============================================================

function insertDeployEvent(deploymentId, eventType, phase, message, data) {
  db.prepare(
    `INSERT INTO deploy_events (deployment_id, event_type, phase, message, data, created_at)
     VALUES (?, ?, ?, ?, ?, ?)`
  ).run(deploymentId, eventType, phase || null, message || null, data ? JSON.stringify(data) : null, Date.now());
}

function getDeployEvents(deploymentId, { since, limit, types } = {}) {
  let sql = `SELECT * FROM deploy_events WHERE deployment_id = ?`;
  const params = [deploymentId];
  if (since) { sql += ` AND created_at > ?`; params.push(since); }
  if (types && types.length > 0) {
    sql += ` AND event_type IN (${types.map(() => '?').join(',')})`;
    params.push(...types);
  }
  sql += ` ORDER BY created_at ASC`;
  if (limit) { sql += ` LIMIT ?`; params.push(limit); }
  return db.prepare(sql).all(...params);
}

function clearDeployEvents(deploymentId) {
  db.prepare(`DELETE FROM deploy_events WHERE deployment_id = ?`).run(deploymentId);
}

/**
 * Generate a unique L2 chain ID that doesn't conflict with existing deployments.
 * Uses range 65537000+ (above the default 65536999) with random offset.
 */
function getNextAvailableL2ChainId() {
  const BASE = 65537000;
  const existing = db.prepare(
    `SELECT chain_id FROM deployments WHERE chain_id IS NOT NULL`
  ).all().map(r => r.chain_id);
  const usedSet = new Set(existing);
  // Also exclude the hardcoded default
  usedSet.add(65536999);

  // Random selection with collision avoidance
  const RANGE = 900000;
  const MAX_RANDOM_ATTEMPTS = 10000;
  let chainId;
  let attempts = 0;
  do {
    chainId = BASE + Math.floor(Math.random() * RANGE);
    attempts++;
  } while (usedSet.has(chainId) && attempts < MAX_RANDOM_ATTEMPTS);

  if (usedSet.has(chainId)) {
    // Fallback: sequential scan for guaranteed uniqueness
    for (let i = 0; i < RANGE; i++) {
      const candidate = BASE + i;
      if (!usedSet.has(candidate)) return candidate;
    }
    throw new Error("No available L2 chain IDs in range");
  }
  return chainId;
}

function getNextAvailableL1ChainId() {
  const BASE = 900;
  const existing = db.prepare(
    `SELECT l1_chain_id FROM deployments WHERE l1_chain_id IS NOT NULL`
  ).all().map(r => r.l1_chain_id);
  const usedSet = new Set(existing);
  usedSet.add(9); // exclude the hardcoded default

  const RANGE = 9000;
  let chainId;
  let attempts = 0;
  do {
    chainId = BASE + Math.floor(Math.random() * RANGE);
    attempts++;
  } while (usedSet.has(chainId) && attempts < 10000);

  if (usedSet.has(chainId)) {
    for (let i = 0; i < RANGE; i++) {
      const candidate = BASE + i;
      if (!usedSet.has(candidate)) return candidate;
    }
    throw new Error("No available L1 chain IDs in range");
  }
  return chainId;
}

function isL1ChainIdTaken(chainId, excludeDeploymentId) {
  const row = db.prepare(
    `SELECT id FROM deployments WHERE l1_chain_id = ? AND id != ?`
  ).get(chainId, excludeDeploymentId);
  return !!row;
}

module.exports = {
  createDeployment, getDeploymentById, getAllDeployments,
  updateDeployment, deleteDeployment, getNextAvailablePorts,
  getNextAvailableL2ChainId, getNextAvailableL1ChainId, isL1ChainIdTaken,
  insertDeployEvent, getDeployEvents, clearDeployEvents,
};

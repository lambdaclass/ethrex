const { v4: uuidv4 } = require("uuid");
const { getDb } = require("./db");

function createDeployment({ userId, programId, name, chainId, rpcUrl, config }) {
  const db = getDb();
  const id = uuidv4();
  const now = Date.now();
  db.prepare(
    `INSERT INTO deployments (id, user_id, program_id, name, chain_id, rpc_url, config, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)`
  ).run(id, userId, programId, name, chainId || null, rpcUrl || null, config ? JSON.stringify(config) : null, now);
  return getDeploymentById(id);
}

function getDeploymentById(id) {
  const db = getDb();
  return db.prepare(
    `SELECT d.*, p.name as program_name, p.program_id as program_slug, p.category
     FROM deployments d
     JOIN programs p ON d.program_id = p.id
     WHERE d.id = ?`
  ).get(id);
}

function getDeploymentsByUser(userId) {
  const db = getDb();
  return db.prepare(
    `SELECT d.*, p.name as program_name, p.program_id as program_slug, p.category
     FROM deployments d
     JOIN programs p ON d.program_id = p.id
     WHERE d.user_id = ?
     ORDER BY d.created_at DESC`
  ).all(userId);
}

function updateDeployment(id, fields) {
  const db = getDb();
  const allowed = ["name", "chain_id", "rpc_url", "status", "config", "docker_project", "l1_port", "l2_port", "proof_coord_port", "phase", "bridge_address", "proposer_address", "error_message", "host_id", "deploy_dir"];
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
  const db = getDb();
  db.prepare("DELETE FROM deployments WHERE id = ?").run(id);
}

function getNextAvailablePorts() {
  const db = getDb();
  const result = db.prepare(
    `SELECT MAX(l1_port) as max_l1, MAX(l2_port) as max_l2, MAX(proof_coord_port) as max_pc FROM deployments WHERE l1_port IS NOT NULL`
  ).get();
  return {
    l1Port: (result.max_l1 || 8544) + 1,
    l2Port: (result.max_l2 || 1728) + 1,
    proofCoordPort: (result.max_pc || 3899) + 1,
  };
}

module.exports = { createDeployment, getDeploymentById, getDeploymentsByUser, updateDeployment, deleteDeployment, getNextAvailablePorts };

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
  return db.prepare("SELECT * FROM deployments WHERE id = ?").get(id);
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
  const allowed = ["name", "chain_id", "rpc_url", "status", "config"];
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

module.exports = { createDeployment, getDeploymentById, getDeploymentsByUser, updateDeployment, deleteDeployment };

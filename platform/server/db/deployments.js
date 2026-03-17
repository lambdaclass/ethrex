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
  const allowed = ["name", "chain_id", "rpc_url", "status", "config", "phase", "bridge_address", "proposer_address",
    "description", "screenshots", "explorer_url", "dashboard_url", "social_links", "l1_chain_id", "network_mode", "owner_wallet"];
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

function getActiveDeploymentById(id) {
  const db = getDb();
  return db.prepare(
    `SELECT d.id, d.user_id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
            d.bridge_address, d.proposer_address, d.created_at,
            d.description, d.screenshots, d.explorer_url, d.dashboard_url,
            d.social_links, d.l1_chain_id, d.network_mode, d.owner_wallet,
            p.name as program_name, p.program_id as program_slug, p.category,
            u.name as owner_name, u.picture as owner_picture
     FROM deployments d
     JOIN programs p ON d.program_id = p.id
     JOIN users u ON d.user_id = u.id
     WHERE d.id = ? AND d.status = 'active'`
  ).get(id);
}

function getActiveDeployments({ limit = 50, offset = 0, search } = {}) {
  const db = getDb();
  let sql = `SELECT d.id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
             d.bridge_address, d.proposer_address, d.created_at,
             d.description, d.screenshots, d.explorer_url, d.dashboard_url,
             d.social_links, d.l1_chain_id, d.network_mode, d.hashtags,
             p.name as program_name, p.program_id as program_slug, p.category,
             u.name as owner_name
             FROM deployments d
             JOIN programs p ON d.program_id = p.id
             JOIN users u ON d.user_id = u.id
             WHERE d.status = 'active'`;
  const params = [];
  if (search) {
    const escaped = search.replace(/[%_\\]/g, '\\$&');
    sql += ` AND (d.name LIKE ? ESCAPE '\\' OR p.name LIKE ? ESCAPE '\\')`;
    params.push(`%${escaped}%`, `%${escaped}%`);
  }
  sql += ` ORDER BY d.created_at DESC LIMIT ? OFFSET ?`;
  params.push(limit, offset);
  return db.prepare(sql).all(...params);
}

function getDeploymentByProposer(proposerAddress, l1ChainId) {
  const db = getDb();
  return db.prepare(
    `SELECT id FROM deployments
     WHERE proposer_address = ? AND l1_chain_id = ? AND status = 'active'`
  ).get(proposerAddress.toLowerCase(), l1ChainId);
}

module.exports = { createDeployment, getDeploymentById, getDeploymentsByUser, updateDeployment, deleteDeployment, getActiveDeployments, getActiveDeploymentById, getDeploymentByProposer };

const { v4: uuidv4 } = require("uuid");
const { getDb } = require("./db");

function createHost({ userId, name, hostname, port, username, authMethod, privateKey }) {
  const db = getDb();
  const id = uuidv4();
  const now = Date.now();
  db.prepare(
    `INSERT INTO hosts (id, user_id, name, hostname, port, username, auth_method, private_key, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`
  ).run(id, userId, name, hostname, port || 22, username, authMethod || "key", privateKey || null, now);
  return getHostById(id);
}

function getHostById(id) {
  const db = getDb();
  return db.prepare("SELECT * FROM hosts WHERE id = ?").get(id);
}

function getHostsByUser(userId) {
  const db = getDb();
  return db.prepare(
    "SELECT id, user_id, name, hostname, port, username, auth_method, status, last_tested, created_at FROM hosts WHERE user_id = ? ORDER BY created_at DESC"
  ).all(userId);
}

function updateHost(id, fields) {
  const db = getDb();
  const allowed = ["name", "hostname", "port", "username", "auth_method", "private_key", "status", "last_tested"];
  const updates = [];
  const values = [];
  for (const [key, value] of Object.entries(fields)) {
    if (allowed.includes(key)) {
      updates.push(`${key} = ?`);
      values.push(value);
    }
  }
  if (updates.length === 0) return getHostById(id);
  values.push(id);
  db.prepare(`UPDATE hosts SET ${updates.join(", ")} WHERE id = ?`).run(...values);
  return getHostById(id);
}

function deleteHost(id) {
  const db = getDb();
  db.prepare("DELETE FROM hosts WHERE id = ?").run(id);
}

module.exports = { createHost, getHostById, getHostsByUser, updateHost, deleteHost };

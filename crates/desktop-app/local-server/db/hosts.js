const { v4: uuidv4 } = require("uuid");
const db = require("./db");

function createHost({ name, hostname, port, username, authMethod, privateKey }) {
  const id = uuidv4();
  const now = Date.now();
  db.prepare(
    `INSERT INTO hosts (id, name, hostname, port, username, auth_method, private_key, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)`
  ).run(id, name, hostname, port || 22, username, authMethod || "key", privateKey || null, now);
  return getHostById(id);
}

function getHostById(id) {
  return db.prepare("SELECT * FROM hosts WHERE id = ?").get(id);
}

function getAllHosts() {
  return db.prepare(
    "SELECT id, name, hostname, port, username, auth_method, status, last_tested, created_at FROM hosts ORDER BY created_at DESC"
  ).all();
}

function updateHost(id, fields) {
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
  db.prepare("DELETE FROM hosts WHERE id = ?").run(id);
}

module.exports = { createHost, getHostById, getAllHosts, updateHost, deleteHost };

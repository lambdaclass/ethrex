const { v4: uuidv4 } = require("uuid");
const { getDb } = require("./db");

function createUser({ email, name, passwordHash, authProvider, picture }) {
  const db = getDb();
  const id = uuidv4();
  const now = Date.now();
  db.prepare(
    `INSERT INTO users (id, email, name, password_hash, auth_provider, picture, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)`
  ).run(id, email, name, passwordHash || null, authProvider || "email", picture || null, now);
  return getUserById(id);
}

function getUserById(id) {
  const db = getDb();
  return db.prepare("SELECT * FROM users WHERE id = ?").get(id);
}

function getUserByEmail(email) {
  const db = getDb();
  return db.prepare("SELECT * FROM users WHERE email = ?").get(email);
}

function findOrCreateOAuthUser({ email, name, picture, authProvider }) {
  let user = getUserByEmail(email);
  if (user) return user;
  return createUser({ email, name, passwordHash: null, authProvider, picture });
}

function updateUser(id, fields) {
  const db = getDb();
  const allowed = ["name", "picture"];
  const updates = [];
  const values = [];

  for (const [key, value] of Object.entries(fields)) {
    if (allowed.includes(key)) {
      updates.push(`${key} = ?`);
      values.push(value);
    }
  }

  if (updates.length === 0) return getUserById(id);

  values.push(id);
  db.prepare(`UPDATE users SET ${updates.join(", ")} WHERE id = ?`).run(...values);
  return getUserById(id);
}

module.exports = { createUser, getUserById, getUserByEmail, findOrCreateOAuthUser, updateUser };

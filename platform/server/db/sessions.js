const { getDb } = require("./db");

const SESSION_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours

function createSession(token, userId) {
  const db = getDb();
  db.prepare("INSERT INTO sessions (token, user_id, created_at) VALUES (?, ?, ?)").run(
    token,
    userId,
    Date.now()
  );
}

function getSession(token) {
  const db = getDb();
  const row = db
    .prepare("SELECT user_id, created_at FROM sessions WHERE token = ?")
    .get(token);
  if (!row) return null;
  if (Date.now() - row.created_at > SESSION_TTL_MS) {
    db.prepare("DELETE FROM sessions WHERE token = ?").run(token);
    return null;
  }
  return { userId: row.user_id, createdAt: row.created_at };
}

function destroySession(token) {
  const db = getDb();
  db.prepare("DELETE FROM sessions WHERE token = ?").run(token);
}

/**
 * Remove all sessions older than SESSION_TTL_MS.
 * Called periodically to prevent stale rows from accumulating.
 */
function cleanupExpiredSessions() {
  const db = getDb();
  const cutoff = Date.now() - SESSION_TTL_MS;
  const result = db.prepare("DELETE FROM sessions WHERE created_at < ?").run(cutoff);
  return result.changes;
}

module.exports = { createSession, getSession, destroySession, cleanupExpiredSessions };

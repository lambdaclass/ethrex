const { v4: uuid } = require("uuid");
const { getDb } = require("./db");

function toggleBookmark(userId, deploymentId) {
  const db = getDb();
  const existing = db
    .prepare("SELECT id FROM bookmarks WHERE user_id = ? AND deployment_id = ?")
    .get(userId, deploymentId);

  if (existing) {
    db.prepare("DELETE FROM bookmarks WHERE id = ?").run(existing.id);
    return { bookmarked: false };
  }

  db.prepare(
    "INSERT INTO bookmarks (id, user_id, deployment_id, created_at) VALUES (?, ?, ?, ?)"
  ).run(uuid(), userId, deploymentId, Date.now());
  return { bookmarked: true };
}

function getUserBookmarks(userId) {
  const db = getDb();
  return db
    .prepare("SELECT deployment_id FROM bookmarks WHERE user_id = ? ORDER BY created_at DESC")
    .all(userId)
    .map((r) => r.deployment_id);
}

function isBookmarked(userId, deploymentId) {
  const db = getDb();
  return !!db
    .prepare("SELECT 1 FROM bookmarks WHERE user_id = ? AND deployment_id = ?")
    .get(userId, deploymentId);
}

module.exports = { toggleBookmark, getUserBookmarks, isBookmarked };

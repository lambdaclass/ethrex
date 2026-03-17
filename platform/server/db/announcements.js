const { v4: uuid } = require("uuid");
const { getDb } = require("./db");

function getAnnouncements(deploymentId) {
  const db = getDb();
  return db
    .prepare(
      `SELECT id, deployment_id, wallet_address, title, content, pinned, created_at
       FROM announcements
       WHERE deployment_id = ?
       ORDER BY created_at DESC
       LIMIT 10`
    )
    .all(deploymentId);
}

function createAnnouncement({ deploymentId, walletAddress, title, content, pinned }) {
  const db = getDb();
  const id = uuid();
  const now = Date.now();
  db.prepare(
    "INSERT INTO announcements (id, deployment_id, wallet_address, title, content, pinned, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
  ).run(id, deploymentId, walletAddress.toLowerCase(), title, content, pinned ? 1 : 0, now);
  return { id, deployment_id: deploymentId, wallet_address: walletAddress.toLowerCase(), title, content, pinned: pinned ? 1 : 0, created_at: now };
}

function deleteAnnouncement(id, walletAddress) {
  const db = getDb();
  const result = db
    .prepare("DELETE FROM announcements WHERE id = ? AND wallet_address = ?")
    .run(id, walletAddress.toLowerCase());
  return result.changes > 0;
}

function getAnnouncementCount(deploymentId) {
  const db = getDb();
  const row = db
    .prepare("SELECT COUNT(*) as count FROM announcements WHERE deployment_id = ?")
    .get(deploymentId);
  return row.count;
}

function updateAnnouncement(id, walletAddress, { title, content, pinned }) {
  const db = getDb();
  const result = db
    .prepare("UPDATE announcements SET title = ?, content = ?, pinned = ? WHERE id = ? AND wallet_address = ?")
    .run(title, content, pinned, id, walletAddress.toLowerCase());
  if (result.changes === 0) return null;
  return db.prepare("SELECT * FROM announcements WHERE id = ?").get(id);
}

module.exports = { getAnnouncements, createAnnouncement, updateAnnouncement, deleteAnnouncement, getAnnouncementCount };

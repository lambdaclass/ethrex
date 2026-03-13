const { v4: uuid } = require("uuid");
const { getDb } = require("./db");

// ── Reviews ──

function getReviewsByDeployment(deploymentId) {
  const db = getDb();
  return db
    .prepare(
      "SELECT * FROM reviews WHERE deployment_id = ? ORDER BY created_at DESC"
    )
    .all(deploymentId);
}

function createReview({ deploymentId, walletAddress, rating, content }) {
  const db = getDb();
  const addr = walletAddress.toLowerCase();
  const id = uuid();
  // Atomic upsert: one review per wallet per deployment
  db.prepare(
    `INSERT INTO reviews (id, deployment_id, wallet_address, rating, content, created_at)
     VALUES (?, ?, ?, ?, ?, ?)
     ON CONFLICT(deployment_id, wallet_address) DO UPDATE SET
       rating = excluded.rating,
       content = excluded.content,
       created_at = excluded.created_at`
  ).run(id, deploymentId, addr, rating, content, Date.now());

  return db
    .prepare(
      "SELECT * FROM reviews WHERE deployment_id = ? AND wallet_address = ?"
    )
    .get(deploymentId, addr);
}

function deleteReview(id, walletAddress) {
  const db = getDb();
  const result = db
    .prepare("DELETE FROM reviews WHERE id = ? AND wallet_address = ?")
    .run(id, walletAddress.toLowerCase());
  return result.changes > 0;
}

// ── Comments ──

function getCommentsByDeployment(deploymentId) {
  const db = getDb();
  // Include deleted comments (deleted_at set) so client can show "삭제된 댓글입니다" placeholder.
  // Content is already cleared on soft-delete, so no data leak.
  return db
    .prepare(
      "SELECT * FROM comments WHERE deployment_id = ? ORDER BY created_at ASC"
    )
    .all(deploymentId);
}

function createComment({ deploymentId, walletAddress, content, parentId }) {
  const db = getDb();
  const id = uuid();
  const addr = walletAddress.toLowerCase();
  db.prepare(
    `INSERT INTO comments (id, deployment_id, wallet_address, content, parent_id, created_at)
     VALUES (?, ?, ?, ?, ?, ?)`
  ).run(id, deploymentId, addr, content, parentId || null, Date.now());

  return db.prepare("SELECT * FROM comments WHERE id = ?").get(id);
}

function deleteComment(id, walletAddress) {
  const db = getDb();
  const result = db
    .prepare("UPDATE comments SET deleted_at = ?, content = '' WHERE id = ? AND wallet_address = ? AND deleted_at IS NULL")
    .run(Date.now(), id, walletAddress.toLowerCase());
  return result.changes > 0;
}

// ── Reactions ──

function toggleReaction({ targetType, targetId, walletAddress }) {
  const db = getDb();
  const addr = walletAddress.toLowerCase();

  // Wrap in transaction for atomicity
  const run = db.transaction(() => {
    const existing = db
      .prepare(
        "SELECT id FROM reactions WHERE target_type = ? AND target_id = ? AND wallet_address = ?"
      )
      .get(targetType, targetId, addr);

    if (existing) {
      db.prepare("DELETE FROM reactions WHERE id = ?").run(existing.id);
    } else {
      db.prepare(
        `INSERT INTO reactions (id, target_type, target_id, wallet_address, created_at)
         VALUES (?, ?, ?, ?, ?)`
      ).run(uuid(), targetType, targetId, addr, Date.now());
    }

    const count = db
      .prepare(
        "SELECT COUNT(*) as count FROM reactions WHERE target_type = ? AND target_id = ?"
      )
      .get(targetType, targetId).count;

    return { liked: !existing, count };
  });

  return run();
}

/** Check if a target (review or comment) exists. */
function targetExists(targetType, targetId) {
  const db = getDb();
  const table = targetType === "review" ? "reviews" : "comments";
  return !!db.prepare(`SELECT 1 FROM ${table} WHERE id = ?`).get(targetId);
}

function getReactionCounts(targetType, targetIds) {
  if (targetIds.length === 0) return {};
  const db = getDb();
  const placeholders = targetIds.map(() => "?").join(",");
  const rows = db
    .prepare(
      `SELECT target_id, COUNT(*) as count FROM reactions
       WHERE target_type = ? AND target_id IN (${placeholders})
       GROUP BY target_id`
    )
    .all(targetType, ...targetIds);

  const counts = {};
  for (const id of targetIds) counts[id] = 0;
  for (const row of rows) counts[row.target_id] = row.count;
  return counts;
}

function getUserReactions(targetType, targetIds, walletAddress) {
  if (targetIds.length === 0 || !walletAddress) return [];
  const db = getDb();
  const addr = walletAddress.toLowerCase();
  const placeholders = targetIds.map(() => "?").join(",");
  const rows = db
    .prepare(
      `SELECT target_id FROM reactions
       WHERE target_type = ? AND target_id IN (${placeholders}) AND wallet_address = ?`
    )
    .all(targetType, ...targetIds, addr);
  return rows.map((r) => r.target_id);
}

// ── Aggregate Stats ──

function getSocialStats(deploymentId) {
  const db = getDb();
  const reviewStats = db
    .prepare(
      "SELECT COUNT(*) as review_count, AVG(rating) as avg_rating FROM reviews WHERE deployment_id = ?"
    )
    .get(deploymentId);
  const commentStats = db
    .prepare(
      "SELECT COUNT(*) as comment_count FROM comments WHERE deployment_id = ?"
    )
    .get(deploymentId);

  return {
    avg_rating: reviewStats.avg_rating
      ? Math.round(reviewStats.avg_rating * 10) / 10
      : null,
    review_count: reviewStats.review_count,
    comment_count: commentStats.comment_count,
  };
}

function getSocialStatsBatch(deploymentIds) {
  if (deploymentIds.length === 0) return {};
  const db = getDb();
  const placeholders = deploymentIds.map(() => "?").join(",");

  const reviewRows = db
    .prepare(
      `SELECT deployment_id, COUNT(*) as review_count, AVG(rating) as avg_rating
       FROM reviews WHERE deployment_id IN (${placeholders})
       GROUP BY deployment_id`
    )
    .all(...deploymentIds);

  const commentRows = db
    .prepare(
      `SELECT deployment_id, COUNT(*) as comment_count
       FROM comments WHERE deployment_id IN (${placeholders})
       GROUP BY deployment_id`
    )
    .all(...deploymentIds);

  const stats = {};
  for (const id of deploymentIds) {
    stats[id] = { avg_rating: null, review_count: 0, comment_count: 0 };
  }
  for (const row of reviewRows) {
    stats[row.deployment_id].avg_rating =
      row.avg_rating ? Math.round(row.avg_rating * 10) / 10 : null;
    stats[row.deployment_id].review_count = row.review_count;
  }
  for (const row of commentRows) {
    stats[row.deployment_id].comment_count = row.comment_count;
  }
  return stats;
}

module.exports = {
  getReviewsByDeployment,
  createReview,
  deleteReview,
  getCommentsByDeployment,
  createComment,
  deleteComment,
  toggleReaction,
  targetExists,
  getReactionCounts,
  getUserReactions,
  getSocialStats,
  getSocialStatsBatch,
};

/**
 * Social feature database queries (reviews, comments, reactions, bookmarks, announcements).
 * Ported from platform/server/db/social.js, bookmarks.js, announcements.js
 */
import { sql, sqlRaw, ensureSchema } from "./db";

// ── Reviews ──

export async function getReviewsByDeployment(deploymentId: string) {
  await ensureSchema();
  const { rows } = await sql`
    SELECT * FROM reviews WHERE deployment_id = ${deploymentId} ORDER BY created_at DESC
  `;
  return rows;
}

export async function createReview(params: {
  deploymentId: string;
  walletAddress: string;
  rating: number;
  content: string;
}) {
  await ensureSchema();
  const id = crypto.randomUUID();
  const addr = params.walletAddress.toLowerCase();
  const now = Date.now();
  await sql`
    INSERT INTO reviews (id, deployment_id, wallet_address, rating, content, created_at)
    VALUES (${id}, ${params.deploymentId}, ${addr}, ${params.rating}, ${params.content}, ${now})
    ON CONFLICT (deployment_id, wallet_address) DO UPDATE SET
      rating = EXCLUDED.rating,
      content = EXCLUDED.content,
      created_at = EXCLUDED.created_at
  `;
  const { rows } = await sql`
    SELECT * FROM reviews WHERE deployment_id = ${params.deploymentId} AND wallet_address = ${addr}
  `;
  return rows[0];
}

export async function deleteReview(id: string, walletAddress: string): Promise<boolean> {
  await ensureSchema();
  const { rows } = await sql`
    DELETE FROM reviews WHERE id = ${id} AND wallet_address = ${walletAddress.toLowerCase()} RETURNING id
  `;
  return rows.length > 0;
}

// ── Comments ──

export async function getCommentsByDeployment(deploymentId: string) {
  await ensureSchema();
  const { rows } = await sql`
    SELECT * FROM comments WHERE deployment_id = ${deploymentId} ORDER BY created_at ASC
  `;
  return rows;
}

export async function createComment(params: {
  deploymentId: string;
  walletAddress: string;
  content: string;
  parentId?: string | null;
}) {
  await ensureSchema();
  const id = crypto.randomUUID();
  const addr = params.walletAddress.toLowerCase();
  const now = Date.now();
  await sql`
    INSERT INTO comments (id, deployment_id, wallet_address, content, parent_id, created_at)
    VALUES (${id}, ${params.deploymentId}, ${addr}, ${params.content}, ${params.parentId || null}, ${now})
  `;
  return { id, deployment_id: params.deploymentId, wallet_address: addr, content: params.content, parent_id: params.parentId || null, deleted_at: null, created_at: now };
}

export async function deleteComment(id: string, walletAddress: string): Promise<boolean> {
  await ensureSchema();
  const { rows } = await sql`
    UPDATE comments SET deleted_at = ${Date.now()}, content = ''
    WHERE id = ${id} AND wallet_address = ${walletAddress.toLowerCase()} AND deleted_at IS NULL
    RETURNING id
  `;
  return rows.length > 0;
}

// ── Reactions ──

export async function getReactionCounts(targetType: string, targetIds: string[]): Promise<Record<string, number>> {
  if (targetIds.length === 0) return {};
  await ensureSchema();
  const { rows } = await sqlRaw(
    `SELECT target_id, COUNT(*)::int as count FROM reactions
     WHERE target_type = $1 AND target_id = ANY($2::text[])
     GROUP BY target_id`,
    [targetType, targetIds]
  );
  const counts: Record<string, number> = {};
  for (const r of rows) {
    counts[r.target_id as string] = r.count as number;
  }
  return counts;
}

export async function getUserReactions(targetType: string, targetIds: string[], walletAddress: string): Promise<string[]> {
  if (targetIds.length === 0) return [];
  await ensureSchema();
  const addr = walletAddress.toLowerCase();
  const { rows } = await sqlRaw(
    `SELECT target_id FROM reactions
     WHERE target_type = $1 AND target_id = ANY($2::text[]) AND wallet_address = $3`,
    [targetType, targetIds, addr]
  );
  return rows.map((r) => r.target_id as string);
}

export async function toggleReaction(params: {
  targetType: string;
  targetId: string;
  walletAddress: string;
}): Promise<{ liked: boolean; count: number }> {
  await ensureSchema();
  const addr = params.walletAddress.toLowerCase();

  const { rows: existing } = await sql`
    SELECT id FROM reactions
    WHERE target_type = ${params.targetType} AND target_id = ${params.targetId} AND wallet_address = ${addr}
  `;

  if (existing.length > 0) {
    await sql`DELETE FROM reactions WHERE id = ${existing[0].id}`;
  } else {
    const id = crypto.randomUUID();
    await sql`
      INSERT INTO reactions (id, target_type, target_id, wallet_address, created_at)
      VALUES (${id}, ${params.targetType}, ${params.targetId}, ${addr}, ${Date.now()})
    `;
  }

  const { rows: countRows } = await sql`
    SELECT COUNT(*)::int as count FROM reactions
    WHERE target_type = ${params.targetType} AND target_id = ${params.targetId}
  `;
  return { liked: existing.length === 0, count: (countRows[0]?.count as number) || 0 };
}

export async function targetExists(targetType: string, targetId: string): Promise<boolean> {
  await ensureSchema();
  const table = targetType === "review" ? "reviews" : "comments";
  const { rows } = await sqlRaw(`SELECT 1 FROM ${table} WHERE id = $1`, [targetId]);
  return rows.length > 0;
}

// ── Social Stats ──

export async function getSocialStats(deploymentId: string) {
  await ensureSchema();
  const { rows: reviewRows } = await sql`
    SELECT COUNT(*)::int as review_count, AVG(rating) as avg_rating FROM reviews WHERE deployment_id = ${deploymentId}
  `;
  const { rows: commentRows } = await sql`
    SELECT COUNT(*)::int as comment_count FROM comments WHERE deployment_id = ${deploymentId}
  `;
  const r = reviewRows[0] || {};
  const c = commentRows[0] || {};
  return {
    avg_rating: r.avg_rating ? Math.round(Number(r.avg_rating) * 10) / 10 : null,
    review_count: (r.review_count as number) || 0,
    comment_count: (c.comment_count as number) || 0,
  };
}

export async function getSocialStatsBatch(deploymentIds: string[]): Promise<Record<string, { avg_rating: number | null; review_count: number; comment_count: number }>> {
  if (deploymentIds.length === 0) return {};
  await ensureSchema();

  const { rows: reviewRows } = await sqlRaw(
    `SELECT deployment_id, COUNT(*)::int as review_count, AVG(rating) as avg_rating
     FROM reviews WHERE deployment_id = ANY($1::text[])
     GROUP BY deployment_id`,
    [deploymentIds]
  );
  const { rows: commentRows } = await sqlRaw(
    `SELECT deployment_id, COUNT(*)::int as comment_count
     FROM comments WHERE deployment_id = ANY($1::text[])
     GROUP BY deployment_id`,
    [deploymentIds]
  );

  const result: Record<string, { avg_rating: number | null; review_count: number; comment_count: number }> = {};
  for (const id of deploymentIds) {
    result[id] = { avg_rating: null, review_count: 0, comment_count: 0 };
  }
  for (const r of reviewRows) {
    const id = r.deployment_id as string;
    if (result[id]) {
      result[id].avg_rating = r.avg_rating ? Math.round(Number(r.avg_rating) * 10) / 10 : null;
      result[id].review_count = (r.review_count as number) || 0;
    }
  }
  for (const c of commentRows) {
    const id = c.deployment_id as string;
    if (result[id]) {
      result[id].comment_count = (c.comment_count as number) || 0;
    }
  }
  return result;
}

// ── Bookmarks ──

export async function toggleBookmark(userId: string, deploymentId: string): Promise<{ bookmarked: boolean }> {
  await ensureSchema();
  const { rows: existing } = await sql`
    SELECT id FROM bookmarks WHERE user_id = ${userId} AND deployment_id = ${deploymentId}
  `;
  if (existing.length > 0) {
    await sql`DELETE FROM bookmarks WHERE id = ${existing[0].id}`;
    return { bookmarked: false };
  }
  const id = crypto.randomUUID();
  await sql`
    INSERT INTO bookmarks (id, user_id, deployment_id, created_at)
    VALUES (${id}, ${userId}, ${deploymentId}, ${Date.now()})
  `;
  return { bookmarked: true };
}

export async function getUserBookmarks(userId: string): Promise<string[]> {
  await ensureSchema();
  const { rows } = await sql`
    SELECT deployment_id FROM bookmarks WHERE user_id = ${userId} ORDER BY created_at DESC
  `;
  return rows.map((r) => r.deployment_id as string);
}

// ── Announcements ──

export async function getAnnouncements(deploymentId: string) {
  await ensureSchema();
  const { rows } = await sql`
    SELECT id, deployment_id, wallet_address, title, content, pinned, created_at
    FROM announcements WHERE deployment_id = ${deploymentId}
    ORDER BY created_at DESC LIMIT 10
  `;
  return rows;
}

export async function createAnnouncement(params: {
  deploymentId: string;
  walletAddress: string;
  title: string;
  content: string;
  pinned: boolean;
}) {
  await ensureSchema();
  const id = crypto.randomUUID();
  const addr = params.walletAddress.toLowerCase();
  const now = Date.now();
  const pinned = params.pinned ? 1 : 0;
  await sql`
    INSERT INTO announcements (id, deployment_id, wallet_address, title, content, pinned, created_at)
    VALUES (${id}, ${params.deploymentId}, ${addr}, ${params.title}, ${params.content}, ${pinned}, ${now})
  `;
  return { id, deployment_id: params.deploymentId, wallet_address: addr, title: params.title, content: params.content, pinned, created_at: now };
}

export async function updateAnnouncement(id: string, walletAddress: string, fields: { title: string; content: string; pinned: boolean }) {
  await ensureSchema();
  const pinned = fields.pinned ? 1 : 0;
  const { rows } = await sql`
    UPDATE announcements SET title = ${fields.title}, content = ${fields.content}, pinned = ${pinned}
    WHERE id = ${id} AND wallet_address = ${walletAddress.toLowerCase()}
    RETURNING *
  `;
  return rows[0] || null;
}

export async function deleteAnnouncement(id: string, walletAddress: string): Promise<boolean> {
  await ensureSchema();
  const { rows } = await sql`
    DELETE FROM announcements WHERE id = ${id} AND wallet_address = ${walletAddress.toLowerCase()} RETURNING id
  `;
  return rows.length > 0;
}

export async function getAnnouncementCount(deploymentId: string): Promise<number> {
  await ensureSchema();
  const { rows } = await sql`
    SELECT COUNT(*)::int as count FROM announcements WHERE deployment_id = ${deploymentId}
  `;
  return (rows[0]?.count as number) || 0;
}

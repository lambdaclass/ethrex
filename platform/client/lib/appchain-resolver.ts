/**
 * Resolve appchain by ID — checks explore_listings first, then legacy deployments.
 * Ported from platform/server/routes/store.js resolveAppchain + getActiveDeployments.
 */
import { sql, sqlRaw, ensureSchema } from "./db";

export async function resolveAppchain(id: string) {
  await ensureSchema();
  // Check listings first
  const { rows: listings } = await sql`
    SELECT * FROM explore_listings WHERE id = ${id} AND status = 'active'
  `;
  if (listings.length > 0) return listings[0];

  // Fallback to legacy deployment
  const { rows: deployments } = await sql`
    SELECT d.id, d.user_id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
           d.bridge_address, d.proposer_address, d.created_at,
           d.description, d.screenshots, d.explorer_url, d.dashboard_url,
           d.social_links, d.l1_chain_id, d.network_mode, d.owner_wallet, d.hashtags,
           p.name as program_name, p.program_id as program_slug, p.category,
           u.name as owner_name, u.picture as owner_picture
    FROM deployments d
    JOIN programs p ON d.program_id = p.id
    JOIN users u ON d.user_id = u.id
    WHERE d.id = ${id} AND d.status = 'active'
  `;
  return deployments[0] || null;
}

export async function getActiveDeployments(params: {
  search?: string | null;
  limit?: number;
  offset?: number;
}) {
  await ensureSchema();
  const { search, limit = 50, offset = 0 } = params;

  let query = `SELECT d.id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
         d.bridge_address, d.proposer_address, d.created_at,
         d.description, d.screenshots, d.explorer_url, d.dashboard_url,
         d.social_links, d.l1_chain_id, d.network_mode, d.hashtags,
         p.name as program_name, p.program_id as program_slug, p.category,
         u.name as owner_name
         FROM deployments d
         JOIN programs p ON d.program_id = p.id
         JOIN users u ON d.user_id = u.id
         WHERE d.status = 'active'`;
  const values: unknown[] = [];
  let paramIdx = 1;

  if (search) {
    const pattern = `%${search}%`;
    query += ` AND (d.name ILIKE $${paramIdx} OR p.name ILIKE $${paramIdx + 1})`;
    values.push(pattern, pattern);
    paramIdx += 2;
  }

  query += ` ORDER BY d.created_at DESC LIMIT $${paramIdx} OFFSET $${paramIdx + 1}`;
  values.push(limit, offset);

  const { rows } = await sqlRaw(query, values);
  return rows;
}

export async function getListings(params: {
  search?: string | null;
  stackType?: string | null;
  l1ChainId?: string | null;
  limit?: number;
  offset?: number;
}) {
  await ensureSchema();
  const { search, stackType, l1ChainId, limit = 50, offset = 0 } = params;

  let query = `SELECT * FROM explore_listings WHERE status = 'active'`;
  const values: unknown[] = [];
  let paramIdx = 1;

  if (search) {
    const pattern = `%${search}%`;
    query += ` AND (name ILIKE $${paramIdx} OR operator_name ILIKE $${paramIdx + 1})`;
    values.push(pattern, pattern);
    paramIdx += 2;
  }
  if (stackType) {
    query += ` AND stack_type = $${paramIdx}`;
    values.push(stackType);
    paramIdx++;
  }
  if (l1ChainId) {
    query += ` AND l1_chain_id = $${paramIdx}`;
    values.push(parseInt(l1ChainId));
    paramIdx++;
  }

  query += ` ORDER BY created_at DESC LIMIT $${paramIdx} OFFSET $${paramIdx + 1}`;
  values.push(limit, offset);

  const { rows } = await sqlRaw(query, values);
  return rows;
}

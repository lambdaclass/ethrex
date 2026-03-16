/**
 * Postgres database connection and schema management.
 * Works with both local PostgreSQL and Neon/Vercel Postgres.
 *
 * Env: DATABASE_URL (postgres connection string)
 */
import { Pool } from "pg";

let pool: Pool | null = null;

function getPool(): Pool {
  if (!pool) {
    const url = process.env.DATABASE_URL;
    if (!url) throw new Error("DATABASE_URL environment variable is not set");
    pool = new Pool({
      connectionString: url,
      ssl: url.includes("neon.tech") || url.includes("vercel-storage") ? true : undefined,
      max: 10,
    });
  }
  return pool;
}

export { getPool };

/**
 * Execute a raw parameterized SQL query (non-tagged-template).
 * Usage: await sqlRaw("SELECT * FROM foo WHERE id = $1", [id])
 */
export async function sqlRaw(
  query: string,
  values: unknown[] = []
): Promise<{ rows: Record<string, unknown>[] }> {
  const result = await getPool().query(query, values);
  return { rows: result.rows };
}

/**
 * Tagged template helper for parameterized SQL queries.
 * Usage: await sql`SELECT * FROM users WHERE id = ${id}`
 * Returns: { rows: Record<string, unknown>[] }
 */
export async function sql(
  strings: TemplateStringsArray,
  ...values: unknown[]
): Promise<{ rows: Record<string, unknown>[] }> {
  // Build parameterized query: $1, $2, ...
  let query = "";
  for (let i = 0; i < strings.length; i++) {
    query += strings[i];
    if (i < values.length) {
      query += `$${i + 1}`;
    }
  }
  const result = await getPool().query(query, values);
  return { rows: result.rows };
}

/**
 * Dynamic UPDATE helper. Builds a single UPDATE query for multiple columns.
 * Usage: await sqlUpdate("deployments", id, { name: "foo", chain_id: 123 })
 */
export async function sqlUpdate(
  table: string,
  id: string,
  fields: Record<string, unknown>
): Promise<{ rows: Record<string, unknown>[] }> {
  const keys = Object.keys(fields);
  if (keys.length === 0) {
    return { rows: [] };
  }

  const setClauses = keys.map((key, i) => `${key} = $${i + 1}`);
  const values = keys.map((k) => fields[k]);
  values.push(id);

  const query = `UPDATE ${table} SET ${setClauses.join(", ")} WHERE id = $${values.length} RETURNING *`;
  const result = await getPool().query(query, values);
  return { rows: result.rows };
}

/**
 * Initialize database schema. Called on first request (lazy init).
 */
let initialized = false;

export async function ensureSchema() {
  if (initialized) return;

  await sql`
    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY,
      email TEXT UNIQUE NOT NULL,
      name TEXT NOT NULL,
      password_hash TEXT,
      auth_provider TEXT DEFAULT 'email',
      role TEXT DEFAULT 'user',
      picture TEXT,
      status TEXT DEFAULT 'active',
      daily_ai_limit INTEGER DEFAULT 50000,
      created_at BIGINT NOT NULL
    )
  `;

  await sql`
    CREATE TABLE IF NOT EXISTS programs (
      id TEXT PRIMARY KEY,
      program_id TEXT UNIQUE NOT NULL,
      program_type_id INTEGER UNIQUE,
      creator_id TEXT NOT NULL REFERENCES users(id),
      name TEXT NOT NULL,
      description TEXT,
      category TEXT DEFAULT 'general',
      icon_url TEXT,
      elf_hash TEXT,
      elf_storage_path TEXT,
      vk_sp1 TEXT,
      vk_risc0 TEXT,
      status TEXT DEFAULT 'pending',
      use_count INTEGER DEFAULT 0,
      batch_count INTEGER DEFAULT 0,
      is_official INTEGER DEFAULT 0,
      created_at BIGINT NOT NULL,
      approved_at BIGINT
    )
  `;

  await sql`
    CREATE TABLE IF NOT EXISTS program_versions (
      id TEXT PRIMARY KEY,
      program_id TEXT NOT NULL REFERENCES programs(id),
      version INTEGER NOT NULL,
      elf_hash TEXT NOT NULL,
      elf_storage_path TEXT,
      uploaded_by TEXT NOT NULL REFERENCES users(id),
      created_at BIGINT NOT NULL
    )
  `;

  await sql`
    CREATE TABLE IF NOT EXISTS deployments (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id),
      program_id TEXT NOT NULL REFERENCES programs(id),
      name TEXT NOT NULL,
      chain_id INTEGER,
      rpc_url TEXT,
      status TEXT DEFAULT 'configured',
      config TEXT,
      phase TEXT DEFAULT 'configured',
      bridge_address TEXT,
      proposer_address TEXT,
      created_at BIGINT NOT NULL
    )
  `;

  await sql`
    CREATE TABLE IF NOT EXISTS sessions (
      token TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      created_at BIGINT NOT NULL
    )
  `;

  await sql`CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id)`;
  await sql`CREATE INDEX IF NOT EXISTS idx_programs_status ON programs(status)`;
  await sql`CREATE INDEX IF NOT EXISTS idx_programs_creator ON programs(creator_id)`;
  await sql`CREATE INDEX IF NOT EXISTS idx_deployments_user ON deployments(user_id)`;
  await sql`CREATE INDEX IF NOT EXISTS idx_deployments_program ON deployments(program_id)`;

  // ── Deployments: extra columns for showroom ──
  await addColumnIfMissing("deployments", "description", "TEXT");
  await addColumnIfMissing("deployments", "screenshots", "TEXT");
  await addColumnIfMissing("deployments", "explorer_url", "TEXT");
  await addColumnIfMissing("deployments", "dashboard_url", "TEXT");
  await addColumnIfMissing("deployments", "social_links", "TEXT");
  await addColumnIfMissing("deployments", "l1_chain_id", "INTEGER");
  await addColumnIfMissing("deployments", "network_mode", "TEXT");
  await addColumnIfMissing("deployments", "owner_wallet", "TEXT");
  await addColumnIfMissing("deployments", "hashtags", "TEXT");

  // ── Explore listings (from metadata repo) ──
  await sql`
    CREATE TABLE IF NOT EXISTS explore_listings (
      id TEXT PRIMARY KEY,
      l1_chain_id INTEGER NOT NULL,
      l2_chain_id INTEGER NOT NULL,
      stack_type TEXT NOT NULL,
      identity_contract TEXT NOT NULL,
      name TEXT NOT NULL,
      rollup_type TEXT,
      status TEXT DEFAULT 'active',
      rpc_url TEXT,
      explorer_url TEXT,
      bridge_url TEXT,
      dashboard_url TEXT,
      native_token_type TEXT DEFAULT 'eth',
      native_token_symbol TEXT DEFAULT 'ETH',
      native_token_decimals INTEGER DEFAULT 18,
      native_token_l1_address TEXT,
      l1_contracts TEXT,
      operator_name TEXT,
      operator_website TEXT,
      operator_social_links TEXT,
      description TEXT,
      screenshots TEXT,
      hashtags TEXT,
      signed_by TEXT,
      signature TEXT,
      owner_wallet TEXT,
      repo_file_path TEXT UNIQUE,
      repo_sha TEXT,
      synced_at BIGINT,
      created_at BIGINT NOT NULL
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS idx_listings_identity ON explore_listings(l1_chain_id, stack_type, identity_contract)`;

  // ── Reviews ──
  await sql`
    CREATE TABLE IF NOT EXISTS reviews (
      id TEXT PRIMARY KEY,
      deployment_id TEXT NOT NULL,
      wallet_address TEXT NOT NULL,
      rating INTEGER NOT NULL CHECK(rating >= 1 AND rating <= 5),
      content TEXT NOT NULL,
      created_at BIGINT NOT NULL
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS idx_reviews_deployment ON reviews(deployment_id)`;
  await sql`CREATE UNIQUE INDEX IF NOT EXISTS idx_reviews_unique ON reviews(deployment_id, wallet_address)`;

  // ── Comments ──
  await sql`
    CREATE TABLE IF NOT EXISTS comments (
      id TEXT PRIMARY KEY,
      deployment_id TEXT NOT NULL,
      wallet_address TEXT NOT NULL,
      content TEXT NOT NULL,
      parent_id TEXT REFERENCES comments(id),
      deleted_at BIGINT,
      created_at BIGINT NOT NULL
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS idx_comments_deployment ON comments(deployment_id)`;

  // ── Reactions ──
  await sql`
    CREATE TABLE IF NOT EXISTS reactions (
      id TEXT PRIMARY KEY,
      target_type TEXT NOT NULL CHECK(target_type IN ('review', 'comment')),
      target_id TEXT NOT NULL,
      wallet_address TEXT NOT NULL,
      created_at BIGINT NOT NULL
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS idx_reactions_target ON reactions(target_type, target_id)`;
  await sql`CREATE UNIQUE INDEX IF NOT EXISTS idx_reactions_unique ON reactions(target_type, target_id, wallet_address)`;

  // ── Bookmarks ──
  await sql`
    CREATE TABLE IF NOT EXISTS bookmarks (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      deployment_id TEXT NOT NULL,
      created_at BIGINT NOT NULL
    )
  `;
  await sql`CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_unique ON bookmarks(user_id, deployment_id)`;
  await sql`CREATE INDEX IF NOT EXISTS idx_bookmarks_user ON bookmarks(user_id)`;

  // ── Announcements ──
  await sql`
    CREATE TABLE IF NOT EXISTS announcements (
      id TEXT PRIMARY KEY,
      deployment_id TEXT NOT NULL,
      wallet_address TEXT NOT NULL,
      title TEXT NOT NULL,
      content TEXT NOT NULL,
      pinned INTEGER DEFAULT 0,
      created_at BIGINT NOT NULL
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS idx_announcements_deployment ON announcements(deployment_id)`;

  await seedOfficialPrograms();

  initialized = true;
}

async function addColumnIfMissing(table: string, column: string, type: string) {
  try {
    await getPool().query(`ALTER TABLE ${table} ADD COLUMN ${column} ${type}`);
  } catch {
    // Column already exists — ignore
  }
}

async function seedOfficialPrograms() {
  const { rows: systemRows } = await sql`SELECT 1 FROM users WHERE id = 'system'`;
  if (systemRows.length === 0) {
    await sql`
      INSERT INTO users (id, email, name, password_hash, auth_provider, role, status, created_at)
      VALUES ('system', 'system@gp-store.local', 'System', NULL, 'system', 'admin', 'active', ${Date.now()})
    `;
  }

  const programs = [
    { programId: "evm-l2", typeId: 1, name: "EVM L2", category: "defi", description: "Default Ethereum execution environment. Full EVM compatibility for general-purpose L2 chains." },
    { programId: "zk-dex", typeId: 2, name: "ZK-DEX", category: "defi", description: "Decentralized exchange circuits optimized for on-chain order matching and settlement." },
  ];

  for (const p of programs) {
    const { rows } = await sql`SELECT 1 FROM programs WHERE program_id = ${p.programId}`;
    if (rows.length === 0) {
      const id = crypto.randomUUID();
      const now = Date.now();
      await sql`
        INSERT INTO programs (id, program_id, program_type_id, creator_id, name, description, category, status, is_official, created_at, approved_at)
        VALUES (${id}, ${p.programId}, ${p.typeId}, 'system', ${p.name}, ${p.description}, ${p.category}, 'active', 1, ${now}, ${now})
      `;
    }
  }
}

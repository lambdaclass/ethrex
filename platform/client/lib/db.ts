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

  await seedOfficialPrograms();

  initialized = true;
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
    { programId: "tokamon", typeId: 3, name: "Tokamon", category: "gaming", description: "Gaming application circuits for provable game state transitions and on-chain gaming." },
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

const Database = require("better-sqlite3");
const path = require("path");
const fs = require("fs");
const { randomUUID: uuid } = require("crypto");

const DB_PATH = process.env.DATABASE_URL || path.join(__dirname, "platform.sqlite");

let db = null;

function getDb() {
  if (!db) {
    db = new Database(DB_PATH);
    db.pragma("journal_mode = WAL");
    db.pragma("foreign_keys = ON");
    runMigrations(db);
    seedOfficialPrograms(db);
  }
  return db;
}

function runMigrations(database) {
  const schema = fs.readFileSync(path.join(__dirname, "schema.sql"), "utf-8");
  database.exec(schema);

  // Add deployment engine columns if they don't exist (for existing DBs)
  const deploymentCols = database.prepare("PRAGMA table_info(deployments)").all();
  const colNames = deploymentCols.map((c) => c.name);
  const newCols = [
    { name: "docker_project", type: "TEXT" },
    { name: "l1_port", type: "INTEGER" },
    { name: "l2_port", type: "INTEGER" },
    { name: "phase", type: "TEXT DEFAULT 'configured'" },
    { name: "bridge_address", type: "TEXT" },
    { name: "proposer_address", type: "TEXT" },
    { name: "error_message", type: "TEXT" },
    { name: "host_id", type: "TEXT" },
    { name: "proof_coord_port", type: "INTEGER" },
    { name: "deploy_dir", type: "TEXT" },
    { name: "tools_l1_explorer_port", type: "INTEGER" },
    { name: "tools_l2_explorer_port", type: "INTEGER" },
    { name: "tools_bridge_ui_port", type: "INTEGER" },
    { name: "tools_db_port", type: "INTEGER" },
    { name: "tools_metrics_port", type: "INTEGER" },
    { name: "env_project_id", type: "TEXT" },
    { name: "env_updated_at", type: "INTEGER" },
    // Showroom social features
    { name: "description", type: "TEXT" },
    { name: "screenshots", type: "TEXT" },
    { name: "explorer_url", type: "TEXT" },
    { name: "dashboard_url", type: "TEXT" },
    { name: "social_links", type: "TEXT" },
    { name: "l1_chain_id", type: "INTEGER" },
    { name: "network_mode", type: "TEXT" },
    { name: "owner_wallet", type: "TEXT" },
  ];
  for (const col of newCols) {
    if (!colNames.includes(col.name)) {
      database.exec(`ALTER TABLE deployments ADD COLUMN ${col.name} ${col.type}`);
    }
  }

  // Add explore_listings columns for existing DBs
  const tables = database.prepare("SELECT name FROM sqlite_master WHERE type='table'").all().map((t) => t.name);
  if (tables.includes("explore_listings")) {
    const listingCols = database.prepare("PRAGMA table_info(explore_listings)").all().map((c) => c.name);
    const listingNewCols = [
      { name: "repo_sha", type: "TEXT" },
      { name: "synced_at", type: "INTEGER" },
    ];
    for (const col of listingNewCols) {
      if (!listingCols.includes(col.name)) {
        database.exec(`ALTER TABLE explore_listings ADD COLUMN ${col.name} ${col.type}`);
      }
    }
  }

  // Add deleted_at to comments for soft-delete (existing DBs)
  const commentCols = database.prepare("PRAGMA table_info(comments)").all().map((c) => c.name);
  const commentNewCols = [
    { name: "deleted_at", type: "INTEGER" },
  ];
  for (const col of commentNewCols) {
    if (!commentCols.includes(col.name)) {
      database.exec(`ALTER TABLE comments ADD COLUMN ${col.name} ${col.type}`);
    }
  }

  // Migrate social tables: remove FK constraints on deployment_id (supports listing IDs too)
  // Check if reviews still has FK constraint by inspecting table SQL
  const reviewTableSql = database.prepare(
    "SELECT sql FROM sqlite_master WHERE type='table' AND name='reviews'"
  ).get();
  if (reviewTableSql && reviewTableSql.sql.includes("REFERENCES deployments")) {
    database.exec("PRAGMA foreign_keys = OFF");
    database.exec(`
      CREATE TABLE IF NOT EXISTS reviews_new (
        id TEXT PRIMARY KEY,
        deployment_id TEXT NOT NULL,
        wallet_address TEXT NOT NULL,
        rating INTEGER NOT NULL CHECK(rating >= 1 AND rating <= 5),
        content TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
      INSERT OR IGNORE INTO reviews_new SELECT id, deployment_id, wallet_address, rating, content, created_at FROM reviews;
      DROP TABLE reviews;
      ALTER TABLE reviews_new RENAME TO reviews;
      CREATE INDEX IF NOT EXISTS idx_reviews_deployment ON reviews(deployment_id);
      CREATE UNIQUE INDEX IF NOT EXISTS idx_reviews_unique ON reviews(deployment_id, wallet_address);

      CREATE TABLE IF NOT EXISTS comments_new (
        id TEXT PRIMARY KEY,
        deployment_id TEXT NOT NULL,
        wallet_address TEXT NOT NULL,
        content TEXT NOT NULL,
        parent_id TEXT REFERENCES comments_new(id),
        deleted_at INTEGER,
        created_at INTEGER NOT NULL
      );
      INSERT OR IGNORE INTO comments_new SELECT id, deployment_id, wallet_address, content, parent_id, deleted_at, created_at FROM comments;
      DROP TABLE comments;
      ALTER TABLE comments_new RENAME TO comments;
      CREATE INDEX IF NOT EXISTS idx_comments_deployment ON comments(deployment_id);

      CREATE TABLE IF NOT EXISTS bookmarks_new (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        deployment_id TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
      INSERT OR IGNORE INTO bookmarks_new SELECT id, user_id, deployment_id, created_at FROM bookmarks;
      DROP TABLE bookmarks;
      ALTER TABLE bookmarks_new RENAME TO bookmarks;
      CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_unique ON bookmarks(user_id, deployment_id);
      CREATE INDEX IF NOT EXISTS idx_bookmarks_user ON bookmarks(user_id);

      CREATE TABLE IF NOT EXISTS announcements_new (
        id TEXT PRIMARY KEY,
        deployment_id TEXT NOT NULL,
        wallet_address TEXT NOT NULL,
        title TEXT NOT NULL,
        content TEXT NOT NULL,
        pinned INTEGER DEFAULT 0,
        created_at INTEGER NOT NULL
      );
      INSERT OR IGNORE INTO announcements_new SELECT id, deployment_id, wallet_address, title, content, pinned, created_at FROM announcements;
      DROP TABLE announcements;
      ALTER TABLE announcements_new RENAME TO announcements;
      CREATE INDEX IF NOT EXISTS idx_announcements_deployment ON announcements(deployment_id);
    `);
    database.exec("PRAGMA foreign_keys = ON");
    console.log("[migration] Removed FK constraints from social tables for listing ID support");
  }
}

function seedOfficialPrograms(database) {
  const programs = [
    {
      programId: "evm-l2",
      typeId: 1,
      name: "EVM L2",
      category: "defi",
      description:
        "Default Ethereum execution environment. Full EVM compatibility for general-purpose L2 chains.",
    },
    {
      programId: "zk-dex",
      typeId: 2,
      name: "ZK-DEX",
      category: "defi",
      description:
        "Decentralized exchange circuits optimized for on-chain order matching and settlement.",
    },
  ];

  // Ensure 'system' user exists for creator_id
  const systemUser = database
    .prepare("SELECT 1 FROM users WHERE id = ?")
    .get("system");
  if (!systemUser) {
    database
      .prepare(
        "INSERT INTO users (id, email, name, password_hash, auth_provider, role, status, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
      )
      .run("system", "system@gp-store.local", "System", null, "system", "admin", "active", Date.now());
  }

  for (const p of programs) {
    const exists = database
      .prepare("SELECT 1 FROM programs WHERE program_id = ?")
      .get(p.programId);
    if (!exists) {
      const now = Date.now();
      database
        .prepare(
          `INSERT INTO programs (id, program_id, program_type_id, creator_id, name, description, category, status, is_official, created_at, approved_at)
           VALUES (?, ?, ?, 'system', ?, ?, ?, 'active', 1, ?, ?)`
        )
        .run(uuid(), p.programId, p.typeId, p.name, p.description, p.category, now, now);
    }
  }
}

module.exports = { getDb };

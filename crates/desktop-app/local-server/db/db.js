const Database = require("better-sqlite3");
const path = require("path");
const fs = require("fs");
const os = require("os");

// DB path: ~/.tokamak-appchain/local.sqlite (or TOKAMAK_DATA_DIR for testing)
const DATA_DIR = process.env.TOKAMAK_DATA_DIR || path.join(os.homedir(), ".tokamak-appchain");
const DB_PATH = path.join(DATA_DIR, "local.sqlite");

// Ensure data directory exists
if (!fs.existsSync(DATA_DIR)) {
  fs.mkdirSync(DATA_DIR, { recursive: true });
}

const db = new Database(DB_PATH);

// Enable WAL mode for better concurrent access
db.pragma("journal_mode = WAL");
db.pragma("foreign_keys = ON");

// Initialize schema
const schema = fs.readFileSync(path.join(__dirname, "schema.sql"), "utf-8");
db.exec(schema);

// Migrations — add columns that may not exist in older DBs
const migrations = [
  "ALTER TABLE deployments ADD COLUMN hashtags TEXT",
  "ALTER TABLE deployments ADD COLUMN timelock_address TEXT",
  "ALTER TABLE deployments ADD COLUMN sp1_verifier_address TEXT",
  "ALTER TABLE deployments ADD COLUMN ever_running INTEGER DEFAULT 0",
  "ALTER TABLE deployments ADD COLUMN guest_program_registry_address TEXT",
  "ALTER TABLE deployments ADD COLUMN verification_status TEXT",
  "ALTER TABLE deployments ADD COLUMN is_public INTEGER DEFAULT 0",
  "ALTER TABLE deployments ADD COLUMN public_domain TEXT",
  "ALTER TABLE deployments ADD COLUMN public_l2_rpc_url TEXT",
  "ALTER TABLE deployments ADD COLUMN public_l2_explorer_url TEXT",
  "ALTER TABLE deployments ADD COLUMN public_l1_explorer_url TEXT",
  "ALTER TABLE deployments ADD COLUMN public_dashboard_url TEXT",
];
for (const sql of migrations) {
  try { db.exec(sql); } catch (e) { if (!e.message.includes('duplicate column name')) { console.error(`Migration failed for "${sql}":`, e); } }
}

module.exports = db;

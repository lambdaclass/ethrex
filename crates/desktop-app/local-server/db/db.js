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

module.exports = db;

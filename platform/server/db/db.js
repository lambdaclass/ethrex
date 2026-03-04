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
    {
      programId: "tokamon",
      typeId: 3,
      name: "Tokamon",
      category: "gaming",
      description:
        "Gaming application circuits for provable game state transitions and on-chain gaming.",
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

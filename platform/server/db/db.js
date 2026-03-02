const Database = require("better-sqlite3");
const path = require("path");
const fs = require("fs");

const DB_PATH = process.env.DATABASE_URL || path.join(__dirname, "platform.sqlite");

let db = null;

function getDb() {
  if (!db) {
    db = new Database(DB_PATH);
    db.pragma("journal_mode = WAL");
    db.pragma("foreign_keys = ON");
    runMigrations(db);
  }
  return db;
}

function runMigrations(database) {
  const schema = fs.readFileSync(path.join(__dirname, "schema.sql"), "utf-8");
  database.exec(schema);
}

module.exports = { getDb };

const { v4: uuidv4 } = require("uuid");
const { getDb } = require("./db");

const RESERVED_TYPE_IDS = 9; // 1-9 reserved for official templates

function createProgram({ programId, creatorId, name, description, category }) {
  const db = getDb();
  const id = uuidv4();
  const now = Date.now();

  db.prepare(
    `INSERT INTO programs (id, program_id, creator_id, name, description, category, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)`
  ).run(id, programId, creatorId, name, description || null, category || "general", now);

  return getProgramById(id);
}

function getProgramById(id) {
  const db = getDb();
  return db.prepare("SELECT * FROM programs WHERE id = ?").get(id);
}

function getProgramByProgramId(programId) {
  const db = getDb();
  return db.prepare("SELECT * FROM programs WHERE program_id = ?").get(programId);
}

function getActivePrograms({ category, search, limit, offset } = {}) {
  const db = getDb();
  let sql = "SELECT * FROM programs WHERE status = 'active'";
  const params = [];

  if (category) {
    sql += " AND category = ?";
    params.push(category);
  }
  if (search) {
    sql += " AND (name LIKE ? OR description LIKE ? OR program_id LIKE ?)";
    const pattern = `%${search}%`;
    params.push(pattern, pattern, pattern);
  }

  sql += " ORDER BY use_count DESC, created_at DESC";
  sql += ` LIMIT ? OFFSET ?`;
  params.push(limit || 50, offset || 0);

  return db.prepare(sql).all(...params);
}

function getProgramsByCreator(creatorId) {
  const db = getDb();
  return db
    .prepare("SELECT * FROM programs WHERE creator_id = ? ORDER BY created_at DESC")
    .all(creatorId);
}

function getAllPrograms({ status } = {}) {
  const db = getDb();
  if (status) {
    return db
      .prepare("SELECT * FROM programs WHERE status = ? ORDER BY created_at DESC")
      .all(status);
  }
  return db.prepare("SELECT * FROM programs ORDER BY created_at DESC").all();
}

function updateProgram(id, fields) {
  const db = getDb();
  const allowed = ["name", "description", "category", "icon_url", "elf_hash", "elf_storage_path", "vk_sp1", "vk_risc0", "status"];
  const updates = [];
  const values = [];

  for (const [key, value] of Object.entries(fields)) {
    if (allowed.includes(key)) {
      updates.push(`${key} = ?`);
      values.push(value);
    }
  }

  if (updates.length === 0) return getProgramById(id);

  values.push(id);
  db.prepare(`UPDATE programs SET ${updates.join(", ")} WHERE id = ?`).run(...values);
  return getProgramById(id);
}

function approveProgram(id) {
  const db = getDb();
  // Assign next available programTypeId
  const maxRow = db.prepare("SELECT MAX(program_type_id) as max_id FROM programs").get();
  const nextTypeId = Math.max((maxRow.max_id || RESERVED_TYPE_IDS) + 1, RESERVED_TYPE_IDS + 1);

  db.prepare(
    "UPDATE programs SET status = 'active', program_type_id = ?, approved_at = ? WHERE id = ?"
  ).run(nextTypeId, Date.now(), id);

  return getProgramById(id);
}

function rejectProgram(id) {
  const db = getDb();
  db.prepare("UPDATE programs SET status = 'rejected' WHERE id = ?").run(id);
  return getProgramById(id);
}

function incrementUseCount(id) {
  const db = getDb();
  db.prepare("UPDATE programs SET use_count = use_count + 1 WHERE id = ?").run(id);
}

function getCategories() {
  const db = getDb();
  const rows = db
    .prepare("SELECT DISTINCT category FROM programs WHERE status = 'active' ORDER BY category")
    .all();
  return rows.map((r) => r.category);
}

module.exports = {
  createProgram,
  getProgramById,
  getProgramByProgramId,
  getActivePrograms,
  getProgramsByCreator,
  getAllPrograms,
  updateProgram,
  approveProgram,
  rejectProgram,
  incrementUseCount,
  getCategories,
};

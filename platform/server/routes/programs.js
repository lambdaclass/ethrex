const express = require("express");
const router = express.Router();
const multer = require("multer");
const crypto = require("crypto");
const path = require("path");
const fs = require("fs");

const { requireAuth } = require("../middleware/auth");
const { isValidProgramId, isValidName, isValidCategory, sanitizeString, VALID_CATEGORIES } = require("../lib/validate");
const {
  createProgram,
  getProgramById,
  getProgramByProgramId,
  getProgramsByCreator,
  updateProgram,
} = require("../db/programs");

// ELF/VK file upload config
const UPLOAD_DIR = path.join(__dirname, "..", "uploads");
if (!fs.existsSync(UPLOAD_DIR)) {
  fs.mkdirSync(UPLOAD_DIR, { recursive: true });
}

const storage = multer.diskStorage({
  destination: (req, file, cb) => {
    const programDir = path.join(UPLOAD_DIR, req.params.id);
    if (!fs.existsSync(programDir)) {
      fs.mkdirSync(programDir, { recursive: true });
    }
    cb(null, programDir);
  },
  filename: (req, file, cb) => {
    const ext = path.extname(file.originalname);
    const name = file.fieldname + ext;
    cb(null, name);
  },
});

const upload = multer({
  storage,
  limits: { fileSize: 100 * 1024 * 1024 }, // 100MB max
  fileFilter: (req, file, cb) => {
    cb(null, true);
  },
});

// All routes require authentication
router.use(requireAuth);

// POST /api/programs — register new program
router.post("/", (req, res) => {
  try {
    const { programId, name, description, category } = req.body;
    if (!programId || !name) {
      return res.status(400).json({ error: "programId and name are required" });
    }
    if (!isValidProgramId(programId)) {
      return res.status(400).json({ error: "programId must be 3-64 lowercase letters, numbers, and hyphens" });
    }
    if (!isValidName(name)) {
      return res.status(400).json({ error: "Name must be 1-100 characters" });
    }
    if (category && !isValidCategory(category)) {
      return res.status(400).json({ error: `Invalid category. Must be one of: ${VALID_CATEGORIES.join(", ")}` });
    }

    // Check for duplicate programId
    const existing = getProgramByProgramId(programId);
    if (existing) {
      return res.status(409).json({ error: "programId already exists" });
    }

    const program = createProgram({
      programId,
      creatorId: req.user.id,
      name,
      description,
      category,
    });

    res.status(201).json({ program });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/programs — list my programs
router.get("/", (req, res) => {
  try {
    const programs = getProgramsByCreator(req.user.id);
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/programs/:id — get my program detail
router.get("/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.creator_id !== req.user.id) {
      return res.status(404).json({ error: "Program not found" });
    }
    res.json({ program });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/programs/:id — update my program
router.put("/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.creator_id !== req.user.id) {
      return res.status(404).json({ error: "Program not found" });
    }

    // Only allow specific fields to be updated
    const allowedUpdates = {};
    if (req.body.name !== undefined) {
      if (!isValidName(req.body.name)) {
        return res.status(400).json({ error: "Name must be 1-100 characters" });
      }
      allowedUpdates.name = req.body.name.trim();
    }
    if (req.body.description !== undefined) {
      allowedUpdates.description = sanitizeString(req.body.description, 2000);
    }
    if (req.body.category !== undefined) {
      if (!isValidCategory(req.body.category)) {
        return res.status(400).json({ error: `Invalid category. Must be one of: ${VALID_CATEGORIES.join(", ")}` });
      }
      allowedUpdates.category = req.body.category;
    }

    const updated = updateProgram(req.params.id, allowedUpdates);
    res.json({ program: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/programs/:id — deactivate my program
router.delete("/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.creator_id !== req.user.id) {
      return res.status(404).json({ error: "Program not found" });
    }

    const updated = updateProgram(req.params.id, { status: "disabled" });
    res.json({ program: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/programs/:id/upload/elf — upload ELF binary
router.post("/:id/upload/elf", upload.single("elf"), (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.creator_id !== req.user.id) {
      return res.status(404).json({ error: "Program not found" });
    }
    if (!req.file) {
      return res.status(400).json({ error: "No file uploaded" });
    }

    // Compute SHA-256 hash of ELF
    const fileBuffer = fs.readFileSync(req.file.path);
    const hash = crypto.createHash("sha256").update(fileBuffer).digest("hex");

    // Record version history
    const { getDb } = require("../db/db");
    const { v4: uuidv4 } = require("uuid");
    const db = getDb();
    const lastVersion = db.prepare(
      "SELECT MAX(version) as max_v FROM program_versions WHERE program_id = ?"
    ).get(req.params.id);
    const nextVersion = (lastVersion?.max_v || 0) + 1;
    db.prepare(
      `INSERT INTO program_versions (id, program_id, version, elf_hash, elf_storage_path, uploaded_by, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)`
    ).run(uuidv4(), req.params.id, nextVersion, hash, req.file.path, req.user.id, Date.now());

    const updated = updateProgram(req.params.id, {
      elf_hash: hash,
      elf_storage_path: req.file.path,
    });

    res.json({
      program: updated,
      upload: {
        filename: req.file.originalname,
        size: req.file.size,
        hash,
      },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/programs/:id/upload/vk — upload verification key
router.post("/:id/upload/vk", upload.fields([
  { name: "vk_sp1", maxCount: 1 },
  { name: "vk_risc0", maxCount: 1 },
]), (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.creator_id !== req.user.id) {
      return res.status(404).json({ error: "Program not found" });
    }

    const files = req.files;
    if (!files || ((!files.vk_sp1 || files.vk_sp1.length === 0) && (!files.vk_risc0 || files.vk_risc0.length === 0))) {
      return res.status(400).json({ error: "No VK files uploaded. Use field names: vk_sp1, vk_risc0" });
    }

    const updates = {};
    const uploadInfo = {};

    if (files.vk_sp1 && files.vk_sp1.length > 0) {
      const buf = fs.readFileSync(files.vk_sp1[0].path);
      updates.vk_sp1 = crypto.createHash("sha256").update(buf).digest("hex");
      uploadInfo.vk_sp1 = { filename: files.vk_sp1[0].originalname, size: files.vk_sp1[0].size, hash: updates.vk_sp1 };
    }
    if (files.vk_risc0 && files.vk_risc0.length > 0) {
      const buf = fs.readFileSync(files.vk_risc0[0].path);
      updates.vk_risc0 = crypto.createHash("sha256").update(buf).digest("hex");
      uploadInfo.vk_risc0 = { filename: files.vk_risc0[0].originalname, size: files.vk_risc0[0].size, hash: updates.vk_risc0 };
    }

    const updated = updateProgram(req.params.id, updates);
    res.json({ program: updated, upload: uploadInfo });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/programs/:id/versions — list ELF version history
router.get("/:id/versions", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.creator_id !== req.user.id) {
      return res.status(404).json({ error: "Program not found" });
    }
    const { getDb } = require("../db/db");
    const db = getDb();
    const versions = db.prepare(
      "SELECT * FROM program_versions WHERE program_id = ? ORDER BY version DESC"
    ).all(req.params.id);
    res.json({ versions });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

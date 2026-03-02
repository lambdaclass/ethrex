const express = require("express");
const router = express.Router();

const { requireAdmin } = require("../middleware/auth");
const { getAllPrograms, approveProgram, rejectProgram, getProgramById } = require("../db/programs");
const { getUserById } = require("../db/users");
const { getDb } = require("../db/db");

// All routes require admin role
router.use(requireAdmin);

// GET /api/admin/programs — all programs (including pending)
router.get("/programs", (req, res) => {
  try {
    const { status } = req.query;
    const programs = getAllPrograms({ status });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/admin/programs/:id — program detail with creator info
router.get("/programs/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program) return res.status(404).json({ error: "Program not found" });

    const creator = getUserById(program.creator_id);
    res.json({
      program,
      creator: creator
        ? { id: creator.id, email: creator.email, name: creator.name, role: creator.role }
        : null,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/admin/programs/:id/approve — approve a program
router.put("/programs/:id/approve", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program) return res.status(404).json({ error: "Program not found" });
    if (program.status !== "pending") {
      return res.status(400).json({ error: `Cannot approve program with status '${program.status}'` });
    }

    const approved = approveProgram(req.params.id);
    res.json({ program: approved });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/admin/programs/:id/reject — reject a program
router.put("/programs/:id/reject", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program) return res.status(404).json({ error: "Program not found" });

    const rejected = rejectProgram(req.params.id);
    res.json({ program: rejected });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/admin/stats — platform stats
router.get("/stats", (req, res) => {
  try {
    const db = getDb();
    const userCount = db.prepare("SELECT COUNT(*) as count FROM users").get().count;
    const programCount = db.prepare("SELECT COUNT(*) as count FROM programs").get().count;
    const activeCount = db.prepare("SELECT COUNT(*) as count FROM programs WHERE status = 'active'").get().count;
    const pendingCount = db.prepare("SELECT COUNT(*) as count FROM programs WHERE status = 'pending'").get().count;

    const deploymentCount = db.prepare("SELECT COUNT(*) as count FROM deployments").get().count;
    res.json({ users: userCount, programs: programCount, active: activeCount, pending: pendingCount, deployments: deploymentCount });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/admin/users — all users
router.get("/users", (req, res) => {
  try {
    const db = getDb();
    const users = db
      .prepare("SELECT id, email, name, role, auth_provider, status, created_at FROM users ORDER BY created_at DESC")
      .all();
    res.json({ users });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/admin/users/:id/role — change user role
router.put("/users/:id/role", (req, res) => {
  try {
    const { role } = req.body;
    if (!role || !["user", "admin"].includes(role)) {
      return res.status(400).json({ error: "role must be 'user' or 'admin'" });
    }
    const db = getDb();
    const user = db.prepare("SELECT * FROM users WHERE id = ?").get(req.params.id);
    if (!user) return res.status(404).json({ error: "User not found" });

    db.prepare("UPDATE users SET role = ? WHERE id = ?").run(role, req.params.id);
    const updated = db.prepare("SELECT id, email, name, role, auth_provider, status, created_at FROM users WHERE id = ?").get(req.params.id);
    res.json({ user: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/admin/users/:id/suspend — suspend user
router.put("/users/:id/suspend", (req, res) => {
  try {
    const db = getDb();
    const user = db.prepare("SELECT * FROM users WHERE id = ?").get(req.params.id);
    if (!user) return res.status(404).json({ error: "User not found" });

    db.prepare("UPDATE users SET status = 'suspended' WHERE id = ?").run(req.params.id);
    const updated = db.prepare("SELECT id, email, name, role, auth_provider, status, created_at FROM users WHERE id = ?").get(req.params.id);
    res.json({ user: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/admin/deployments — all deployments
router.get("/deployments", (req, res) => {
  try {
    const db = getDb();
    const deployments = db.prepare(
      `SELECT d.*, p.name as program_name, p.program_id as program_slug,
              u.name as user_name, u.email as user_email
       FROM deployments d
       JOIN programs p ON d.program_id = p.id
       JOIN users u ON d.user_id = u.id
       ORDER BY d.created_at DESC`
    ).all();
    res.json({ deployments });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/admin/users/:id/activate — activate user
router.put("/users/:id/activate", (req, res) => {
  try {
    const db = getDb();
    const user = db.prepare("SELECT * FROM users WHERE id = ?").get(req.params.id);
    if (!user) return res.status(404).json({ error: "User not found" });

    db.prepare("UPDATE users SET status = 'active' WHERE id = ?").run(req.params.id);
    const updated = db.prepare("SELECT id, email, name, role, auth_provider, status, created_at FROM users WHERE id = ?").get(req.params.id);
    res.json({ user: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

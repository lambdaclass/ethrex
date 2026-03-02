const express = require("express");
const router = express.Router();

const { getActivePrograms, getProgramById, getCategories } = require("../db/programs");

// GET /api/store/programs — public program listing
router.get("/programs", (req, res) => {
  try {
    const { category, search, limit, offset } = req.query;
    const programs = getActivePrograms({
      category,
      search,
      limit: parseInt(limit) || 50,
      offset: parseInt(offset) || 0,
    });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/programs/:id — program detail
router.get("/programs/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.status !== "active") {
      return res.status(404).json({ error: "Program not found" });
    }
    res.json({ program });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/categories — category list
router.get("/categories", (req, res) => {
  try {
    const categories = getCategories();
    res.json({ categories });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/featured — featured programs (top by usage)
router.get("/featured", (req, res) => {
  try {
    const programs = getActivePrograms({ limit: 6 });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

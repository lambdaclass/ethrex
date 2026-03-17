const express = require("express");
const bcrypt = require("bcryptjs");
const router = express.Router();

const { createUser, getUserByEmail, findOrCreateOAuthUser, updateUser } = require("../db/users");
const { createSession, destroySession, requireAuth } = require("../middleware/auth");
const { verifyGoogleIdToken, isGoogleAuthConfigured, getGoogleClientId } = require("../lib/google-auth");
const { exchangeNaverCode, isNaverAuthConfigured, getNaverClientId } = require("../lib/naver-auth");
const { exchangeKakaoCode, isKakaoAuthConfigured, getKakaoClientId } = require("../lib/kakao-auth");
const { isValidEmail, isValidPassword, isValidName } = require("../lib/validate");

// GET /api/auth/providers — available auth providers
router.get("/providers", (req, res) => {
  res.json({
    google: isGoogleAuthConfigured(),
    naver: isNaverAuthConfigured(),
    kakao: isKakaoAuthConfigured(),
  });
});

// GET /api/auth/google-client-id
router.get("/google-client-id", (req, res) => {
  const clientId = getGoogleClientId();
  if (!clientId) return res.status(404).json({ error: "Google auth not configured" });
  res.json({ clientId });
});

// GET /api/auth/naver-client-id
router.get("/naver-client-id", (req, res) => {
  const clientId = getNaverClientId();
  if (!clientId) return res.status(404).json({ error: "Naver auth not configured" });
  res.json({ clientId });
});

// GET /api/auth/kakao-client-id
router.get("/kakao-client-id", (req, res) => {
  const clientId = getKakaoClientId();
  if (!clientId) return res.status(404).json({ error: "Kakao auth not configured" });
  res.json({ clientId });
});

// POST /api/auth/signup — email/password signup
router.post("/signup", async (req, res) => {
  try {
    const { email, password, name } = req.body;
    if (!email || !password || !name) {
      return res.status(400).json({ error: "email, password, and name are required" });
    }
    if (!isValidEmail(email)) {
      return res.status(400).json({ error: "Invalid email format" });
    }
    if (!isValidPassword(password)) {
      return res.status(400).json({ error: "Password must be 8-128 characters" });
    }
    if (!isValidName(name)) {
      return res.status(400).json({ error: "Name must be 1-100 characters" });
    }

    const existing = getUserByEmail(email);
    if (existing) {
      return res.status(409).json({ error: "Email already registered" });
    }

    const passwordHash = await bcrypt.hash(password, 10);
    const user = createUser({ email, name, passwordHash, authProvider: "email" });
    const token = createSession(user.id);

    res.status(201).json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/auth/login — email/password login
router.post("/login", async (req, res) => {
  try {
    const { email, password } = req.body;
    if (!email || !password) {
      return res.status(400).json({ error: "email and password are required" });
    }

    const user = getUserByEmail(email);
    if (!user || !user.password_hash) {
      return res.status(401).json({ error: "Invalid credentials" });
    }

    const valid = await bcrypt.compare(password, user.password_hash);
    if (!valid) {
      return res.status(401).json({ error: "Invalid credentials" });
    }

    if (user.status !== "active") {
      return res.status(403).json({ error: "Account is suspended" });
    }

    const token = createSession(user.id);
    res.json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/auth/google — Google OAuth
router.post("/google", async (req, res) => {
  try {
    const { idToken } = req.body;
    if (!idToken) return res.status(400).json({ error: "idToken is required" });

    const profile = await verifyGoogleIdToken(idToken);
    const user = findOrCreateOAuthUser({ ...profile, authProvider: "google" });
    const token = createSession(user.id);

    res.json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e) {
    res.status(401).json({ error: e.message });
  }
});

// POST /api/auth/naver — Naver OAuth
router.post("/naver", async (req, res) => {
  try {
    const { code, state } = req.body;
    if (!code) return res.status(400).json({ error: "code is required" });

    const profile = await exchangeNaverCode(code, state);
    const user = findOrCreateOAuthUser({ ...profile, authProvider: "naver" });
    const token = createSession(user.id);

    res.json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e) {
    res.status(401).json({ error: e.message });
  }
});

// POST /api/auth/kakao — Kakao OAuth
router.post("/kakao", async (req, res) => {
  try {
    const { code, redirectUri } = req.body;
    if (!code) return res.status(400).json({ error: "code is required" });

    const profile = await exchangeKakaoCode(code, redirectUri);
    const user = findOrCreateOAuthUser({ ...profile, authProvider: "kakao" });
    const token = createSession(user.id);

    res.json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e) {
    res.status(401).json({ error: e.message });
  }
});

// GET /api/auth/me — current user info
router.get("/me", requireAuth, (req, res) => {
  const { id, email, name, role, picture, auth_provider } = req.user;
  res.json({ id, email, name, role, picture, authProvider: auth_provider });
});

// PUT /api/auth/profile — update user profile
router.put("/profile", requireAuth, (req, res) => {
  try {
    const { name } = req.body;
    if (!isValidName(name)) {
      return res.status(400).json({ error: "Name must be 1-100 characters" });
    }

    const updated = updateUser(req.user.id, { name: name.trim() });
    const { id, email, role, picture, auth_provider } = updated;
    res.json({
      user: { id, email, name: updated.name, role, picture, authProvider: auth_provider },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/auth/logout
router.post("/logout", requireAuth, (req, res) => {
  destroySession(req.sessionToken);
  res.json({ ok: true });
});

module.exports = router;

const crypto = require("crypto");
const { getUserById } = require("../db/users");
const sessions = require("../db/sessions");

function createSession(userId) {
  const token = "ps_" + crypto.randomBytes(32).toString("hex");
  sessions.createSession(token, userId);
  return token;
}

function destroySession(token) {
  sessions.destroySession(token);
}

// Middleware: require authentication
async function requireAuth(req, res, next) {
  try {
    const bearer = req.headers["authorization"]?.replace("Bearer ", "");
    if (!bearer) {
      return res.status(401).json({ error: "Authentication required" });
    }

    const session = sessions.getSession(bearer);
    if (!session) {
      return res.status(401).json({ error: "Invalid or expired session" });
    }

    const user = getUserById(session.userId);
    if (!user) {
      sessions.destroySession(bearer);
      return res.status(401).json({ error: "User not found" });
    }
    if (user.status !== "active") {
      return res.status(403).json({ error: "Account is suspended" });
    }

    req.user = user;
    req.sessionToken = bearer;
    next();
  } catch (e) {
    next(e);
  }
}

// Middleware: require admin role
async function requireAdmin(req, res, next) {
  try {
    const bearer = req.headers["authorization"]?.replace("Bearer ", "");
    if (!bearer) {
      return res.status(401).json({ error: "Authentication required" });
    }

    const session = sessions.getSession(bearer);
    if (!session) {
      return res.status(401).json({ error: "Invalid or expired session" });
    }

    const user = getUserById(session.userId);
    if (!user || user.role !== "admin") {
      return res.status(403).json({ error: "Admin access required" });
    }

    req.user = user;
    req.sessionToken = bearer;
    next();
  } catch (e) {
    next(e);
  }
}

module.exports = { createSession, destroySession, requireAuth, requireAdmin };

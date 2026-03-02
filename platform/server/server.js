require("dotenv").config();

const express = require("express");
const cors = require("cors");

const authRoutes = require("./routes/auth");
const storeRoutes = require("./routes/store");
const programRoutes = require("./routes/programs");
const adminRoutes = require("./routes/admin");
const deploymentRoutes = require("./routes/deployments");

const app = express();
const PORT = process.env.PORT || 5001;

// Middleware
app.use(cors({
  origin: process.env.CORS_ORIGINS
    ? process.env.CORS_ORIGINS.split(",")
    : ["http://localhost:3000", "http://localhost:3001"],
  credentials: true,
}));
app.use(express.json({ limit: "50mb" }));

// Simple rate limiting (per IP, in-memory)
const rateLimit = new Map();
const RATE_LIMIT_WINDOW = 60 * 1000; // 1 minute
const RATE_LIMIT_MAX = 100; // requests per minute

app.use((req, res, next) => {
  const ip = req.ip || req.connection.remoteAddress;
  const now = Date.now();
  const record = rateLimit.get(ip);

  if (!record || now - record.windowStart > RATE_LIMIT_WINDOW) {
    rateLimit.set(ip, { windowStart: now, count: 1 });
    return next();
  }

  record.count++;
  if (record.count > RATE_LIMIT_MAX) {
    return res.status(429).json({ error: "Too many requests. Try again later." });
  }

  next();
});

// Clean up stale rate limit entries every 5 minutes
setInterval(() => {
  const now = Date.now();
  for (const [ip, record] of rateLimit) {
    if (now - record.windowStart > RATE_LIMIT_WINDOW * 2) {
      rateLimit.delete(ip);
    }
  }
}, 5 * 60 * 1000);

// Clean up expired sessions every hour
const { cleanupExpiredSessions } = require("./db/sessions");
setInterval(() => {
  const removed = cleanupExpiredSessions();
  if (removed > 0) {
    console.log(`Cleaned up ${removed} expired session(s)`);
  }
}, 60 * 60 * 1000);

// Static file serving for uploads
const path = require("path");
app.use("/uploads", express.static(path.join(__dirname, "uploads")));

// Routes
app.use("/api/auth", authRoutes);
app.use("/api/store", storeRoutes);
app.use("/api/programs", programRoutes);
app.use("/api/admin", adminRoutes);
app.use("/api/deployments", deploymentRoutes);

// Health check
app.get("/api/health", (req, res) => {
  res.json({ status: "ok", version: "0.1.0" });
});

// Error handler
app.use((err, req, res, _next) => {
  console.error("Unhandled error:", err);
  res.status(500).json({ error: "Internal server error" });
});

app.listen(PORT, () => {
  console.log(`Guest Program Store server running on port ${PORT}`);
});

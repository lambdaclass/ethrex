const express = require("express");
const cors = require("cors");
const path = require("path");

const deploymentRoutes = require("./routes/deployments");
const hostRoutes = require("./routes/hosts");
const fsRoutes = require("./routes/fs");

const app = express();
const PORT = process.env.LOCAL_SERVER_PORT || 5002;

// Middleware
app.use(cors({
  origin: [
    "http://localhost:3002",
    "http://127.0.0.1:3002",
    "http://localhost:1420",  // Tauri dev server
    "http://127.0.0.1:1420",
    "tauri://localhost",      // Tauri production
  ],
  credentials: true,
}));
app.use(express.json());

// Static web UI
app.use(express.static(path.join(__dirname, "public")));

// API Routes
app.use("/api/deployments", deploymentRoutes);
app.use("/api/hosts", hostRoutes);
app.use("/api/fs", fsRoutes);

// Health check
app.get("/api/health", (req, res) => {
  res.json({ status: "ok", version: "0.1.0", type: "local-server" });
});

// Error handler
app.use((err, req, res, _next) => {
  console.error("Unhandled error:", err);
  res.status(500).json({ error: "Internal server error" });
});

// Bind to localhost only (security: no external access)
app.listen(PORT, "127.0.0.1", () => {
  console.log(`Tokamak local server running on http://127.0.0.1:${PORT}`);
});

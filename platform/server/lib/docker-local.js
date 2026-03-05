/**
 * Docker Compose CLI wrapper for local L2 deployments.
 *
 * Each deployment gets its own Docker Compose project name for isolation.
 * Compose files are generated per-deployment in ~/.tokamak/deployments/<id>/
 */

const { spawn, execSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const ETHREX_ROOT = path.resolve(__dirname, "../../..");

function composeCmd(projectName, composeFile, args) {
  return ["docker", "compose", "-p", projectName, "-f", composeFile, ...args];
}

function runCompose(projectName, composeFile, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const [cmd, ...cmdArgs] = composeCmd(projectName, composeFile, args);
    const proc = spawn(cmd, cmdArgs, {
      cwd: ETHREX_ROOT,
      env: { ...process.env, ...(opts.env || {}) },
      stdio: opts.stdio || "pipe",
    });

    let stdout = "";
    let stderr = "";
    if (proc.stdout) proc.stdout.on("data", (d) => {
      stdout += d;
      if (opts.onLog) opts.onLog(d.toString());
    });
    if (proc.stderr) proc.stderr.on("data", (d) => {
      stderr += d;
      if (opts.onLog) opts.onLog(d.toString());
    });

    proc.on("close", (code) => {
      if (code !== 0 && !opts.ignoreError) {
        reject(new Error(`docker compose exited with code ${code}: ${stderr}`));
      } else {
        resolve({ stdout, stderr, code });
      }
    });

    proc.on("error", reject);

    if (opts.timeout) {
      setTimeout(() => {
        proc.kill("SIGTERM");
        reject(new Error("docker compose timed out"));
      }, opts.timeout);
    }
  });
}

/** Build Docker images for the deployment */
async function buildImages(projectName, composeFile, env = {}, onLog) {
  // Remove any existing images for this project to avoid "already exists" errors
  try {
    const existing = execSync(
      `docker images --filter "reference=ethrex:*-${projectName}" --format "{{.Repository}}:{{.Tag}}"`,
      { timeout: 10000 }
    ).toString().trim();
    if (existing) {
      for (const img of existing.split("\n").filter(Boolean)) {
        execSync(`docker rmi "${img}"`, { timeout: 30000, stdio: "ignore" });
      }
    }
  } catch {
    // Ignore cleanup errors — images may be in use or already gone
  }
  return runCompose(projectName, composeFile, ["build"], { env, onLog });
}

/** Start L1 service */
async function startL1(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "tokamak-app-l1"], { env });
}

/** Run contract deployer (waits for completion) */
async function deployContracts(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "tokamak-app-deployer"], {
    env,
    timeout: 600000, // 10 minutes max
  });
}

/** Extract .env from the deployer volume */
async function extractEnv(projectName, composeFile) {
  const result = await runCompose(
    projectName,
    composeFile,
    ["exec", "-T", "tokamak-app-l1", "cat", "/dev/null"],
    { ignoreError: true }
  );

  // Use docker cp to extract the .env from the named volume
  const volumeName = `${projectName}_env`;
  const tempDir = path.join(require("os").tmpdir(), `ethrex-${projectName}`);
  fs.mkdirSync(tempDir, { recursive: true });

  try {
    // Create a temporary container to access the volume
    execSync(
      `docker run --rm -v ${volumeName}:/env -v ${tempDir}:/out alpine cp /env/.env /out/.env`,
      { cwd: ETHREX_ROOT, timeout: 30000 }
    );

    const envContent = fs.readFileSync(path.join(tempDir, ".env"), "utf-8");
    console.log(`[extractEnv] Volume ${volumeName} content:\n${envContent}`);
    const parsed = {};
    for (const line of envContent.split("\n")) {
      const match = line.match(/^([^=]+)=(.*)$/);
      if (match) parsed[match[1].trim()] = match[2].trim();
    }
    console.log(`[extractEnv] Parsed BRIDGE_ADDRESS: ${parsed.ETHREX_WATCHER_BRIDGE_ADDRESS || 'NOT FOUND'}`);
    return parsed;
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

/** Start L2 service (--no-deps: don't restart deployer) */
async function startL2(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "--no-deps", "tokamak-app-l2"], { env });
}

/** Start prover service (--no-deps: don't restart deployer or L2) */
async function startProver(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "--no-deps", "tokamak-app-prover"], { env });
}

/** Stop a single service */
async function stopService(projectName, composeFile, service) {
  return runCompose(projectName, composeFile, ["stop", service], { ignoreError: true });
}

/** Stop all services (keep volumes) */
async function stop(projectName, composeFile) {
  return runCompose(projectName, composeFile, ["stop"], { ignoreError: true });
}

/** Start all stopped services */
async function start(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d"], { env });
}

/** Destroy all services and volumes */
async function destroy(projectName, composeFile) {
  return runCompose(projectName, composeFile, ["down", "--volumes", "--remove-orphans"], {
    ignoreError: true,
  });
}

/** Get container status as JSON */
async function getStatus(projectName, composeFile) {
  try {
    const { stdout } = await runCompose(
      projectName,
      composeFile,
      ["ps", "--format", "json"],
      { ignoreError: true }
    );
    // docker compose ps --format json outputs one JSON per line
    const containers = stdout
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        try {
          return JSON.parse(line);
        } catch {
          return null;
        }
      })
      .filter(Boolean);
    return containers;
  } catch {
    return [];
  }
}

/** Get logs for a service */
async function getLogs(projectName, composeFile, service, tail = 100) {
  const args = ["logs", "--tail", String(tail)];
  if (service) args.push(service);
  const { stdout } = await runCompose(projectName, composeFile, args, { ignoreError: true });
  return stdout;
}

/** Stream logs as a child process (returns the spawned process) */
function streamLogs(projectName, composeFile, service) {
  const args = ["logs", "-f", "--tail", "50"];
  if (service) args.push(service);
  const [cmd, ...cmdArgs] = composeCmd(projectName, composeFile, args);
  return spawn(cmd, cmdArgs, { cwd: ETHREX_ROOT, stdio: "pipe" });
}

/** Start support tools (Blockscout, Bridge UI, Dashboard) using the existing tools compose file */
async function startTools(envVars, toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");

  if (!fs.existsSync(toolsCompose)) {
    throw new Error("Tools compose file not found: " + toolsCompose);
  }

  // Write .zk-dex-deployed.env for tools (Blockscout address names, bridge UI)
  const envPath = path.join(l2Dir, ".zk-dex-deployed.env");
  const envLines = Object.entries(envVars || {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
  fs.writeFileSync(envPath, envLines + "\n");

  // Environment variables for dynamic tool ports
  const toolsEnv = {
    TOOLS_L1_EXPLORER_PORT: String(toolsPorts.toolsL1ExplorerPort || 8083),
    TOOLS_L2_EXPLORER_PORT: String(toolsPorts.toolsL2ExplorerPort || 8082),
    TOOLS_BRIDGE_UI_PORT: String(toolsPorts.toolsBridgeUIPort || 3000),
    TOOLS_DB_PORT: String(toolsPorts.toolsDbPort || 7432),
    TOOLS_L1_RPC_PORT: String(toolsPorts.l1Port || 8545),
    TOOLS_L2_RPC_PORT: String(toolsPorts.l2Port || 1729),
    TOOLS_METRICS_PORT: String(toolsPorts.toolsMetricsPort || 3702),
  };

  // Build bridge UI image
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", ["compose", "-f", toolsCompose, "build"], {
      cwd: l2Dir,
      env: { ...process.env, ...toolsEnv },
      stdio: "pipe",
    });
    let stderr = "";
    if (proc.stderr) proc.stderr.on("data", (d) => (stderr += d));
    proc.on("close", (code) => {
      if (code !== 0) reject(new Error(`Tools build failed: ${stderr.slice(-500)}`));
      else resolve();
    });
    proc.on("error", reject);
  });

  // Start all tools
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", ["compose", "-f", toolsCompose, "up", "-d"], {
      cwd: l2Dir,
      env: { ...process.env, ...toolsEnv },
      stdio: "pipe",
    });
    let stderr = "";
    if (proc.stderr) proc.stderr.on("data", (d) => (stderr += d));
    proc.on("close", (code) => {
      if (code !== 0) reject(new Error(`Tools start failed: ${stderr.slice(-500)}`));
      else resolve();
    });
    proc.on("error", reject);
  });
}

/** Build support tools images only (no start) */
async function buildTools(toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");

  if (!fs.existsSync(toolsCompose)) {
    throw new Error("Tools compose file not found: " + toolsCompose);
  }

  const toolsEnv = {
    TOOLS_L1_EXPLORER_PORT: String(toolsPorts.toolsL1ExplorerPort || 8083),
    TOOLS_L2_EXPLORER_PORT: String(toolsPorts.toolsL2ExplorerPort || 8082),
    TOOLS_BRIDGE_UI_PORT: String(toolsPorts.toolsBridgeUIPort || 3000),
    TOOLS_DB_PORT: String(toolsPorts.toolsDbPort || 7432),
    TOOLS_L1_RPC_PORT: String(toolsPorts.l1Port || 8545),
    TOOLS_L2_RPC_PORT: String(toolsPorts.l2Port || 1729),
    TOOLS_METRICS_PORT: String(toolsPorts.toolsMetricsPort || 3702),
  };

  await new Promise((resolve, reject) => {
    const proc = spawn("docker", ["compose", "-f", toolsCompose, "build"], {
      cwd: l2Dir,
      env: { ...process.env, ...toolsEnv },
      stdio: "pipe",
    });
    let stderr = "";
    if (proc.stderr) proc.stderr.on("data", (d) => (stderr += d));
    proc.on("close", (code) => {
      if (code !== 0) reject(new Error(`Tools build failed: ${stderr.slice(-500)}`));
      else resolve();
    });
    proc.on("error", reject);
  });
}

/** Restart support tools (no rebuild, just stop + up) */
async function restartTools(envVars, toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");

  if (!fs.existsSync(toolsCompose)) {
    throw new Error("Tools compose file not found: " + toolsCompose);
  }

  // Write .zk-dex-deployed.env for tools
  const envPath = path.join(l2Dir, ".zk-dex-deployed.env");
  const envLines = Object.entries(envVars || {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
  fs.writeFileSync(envPath, envLines + "\n");

  const toolsEnv = {
    TOOLS_L1_EXPLORER_PORT: String(toolsPorts.toolsL1ExplorerPort || 8083),
    TOOLS_L2_EXPLORER_PORT: String(toolsPorts.toolsL2ExplorerPort || 8082),
    TOOLS_BRIDGE_UI_PORT: String(toolsPorts.toolsBridgeUIPort || 3000),
    TOOLS_DB_PORT: String(toolsPorts.toolsDbPort || 7432),
    TOOLS_L1_RPC_PORT: String(toolsPorts.l1Port || 8545),
    TOOLS_L2_RPC_PORT: String(toolsPorts.l2Port || 1729),
    TOOLS_METRICS_PORT: String(toolsPorts.toolsMetricsPort || 3702),
  };

  // Stop existing tools
  await new Promise((resolve) => {
    const proc = spawn("docker", ["compose", "-f", toolsCompose, "down", "--remove-orphans"], {
      cwd: l2Dir,
      env: { ...process.env, ...toolsEnv },
      stdio: "pipe",
    });
    proc.on("close", () => resolve());
    proc.on("error", () => resolve());
  });

  // Start without build
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", ["compose", "-f", toolsCompose, "up", "-d"], {
      cwd: l2Dir,
      env: { ...process.env, ...toolsEnv },
      stdio: "pipe",
    });
    let stderr = "";
    if (proc.stderr) proc.stderr.on("data", (d) => (stderr += d));
    proc.on("close", (code) => {
      if (code !== 0) reject(new Error(`Tools restart failed: ${stderr.slice(-500)}`));
      else resolve();
    });
    proc.on("error", reject);
  });
}

/** Get logs for a tools service */
async function getToolsLogs(service, tail = 100) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  if (!fs.existsSync(toolsCompose)) return "";

  const args = ["compose", "-f", toolsCompose, "logs", "--tail", String(tail)];
  if (service) args.push(service);

  return new Promise((resolve) => {
    const proc = spawn("docker", args, { cwd: l2Dir, stdio: "pipe" });
    let stdout = "";
    if (proc.stdout) proc.stdout.on("data", (d) => (stdout += d));
    if (proc.stderr) proc.stderr.on("data", (d) => (stdout += d));
    proc.on("close", () => resolve(stdout));
    proc.on("error", () => resolve(""));
  });
}

/** Stream logs for a tools service (returns spawned process) */
function streamToolsLogs(service) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  const args = ["compose", "-f", toolsCompose, "logs", "-f", "--tail", "50"];
  if (service) args.push(service);
  return spawn("docker", args, { cwd: l2Dir, stdio: "pipe" });
}

/** Stop support tools */
async function stopTools() {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  if (!fs.existsSync(toolsCompose)) return;

  await new Promise((resolve) => {
    const proc = spawn("docker", ["compose", "-f", toolsCompose, "down", "--remove-orphans"], {
      cwd: l2Dir,
      stdio: "pipe",
    });
    proc.on("close", () => resolve());
    proc.on("error", () => resolve());
  });
}

/** Check if Docker daemon is available */
function isDockerAvailable() {
  try {
    execSync("docker info", { stdio: "ignore", timeout: 5000 });
    return true;
  } catch {
    return false;
  }
}

module.exports = {
  buildImages,
  startL1,
  deployContracts,
  extractEnv,
  stopService,
  startL2,
  startProver,
  stop,
  start,
  destroy,
  getStatus,
  getLogs,
  streamLogs,
  isDockerAvailable,
  startTools,
  buildTools,
  restartTools,
  getToolsLogs,
  streamToolsLogs,
  stopTools,
  ETHREX_ROOT,
};

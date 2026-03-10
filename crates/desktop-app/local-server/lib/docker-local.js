/**
 * Docker Compose CLI wrapper for local L2 deployments.
 *
 * Each deployment gets its own Docker Compose project name for isolation.
 * Compose files are generated per-deployment in ~/.tokamak/deployments/<id>/
 */

const { spawn, execSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const ETHREX_ROOT = path.resolve(__dirname, "../../../..");

function composeCmd(projectName, composeFile, args) {
  return ["docker", "compose", "-p", projectName, "-f", composeFile, ...args];
}

function runCompose(projectName, composeFile, args, opts = {}) {
  const [cmd, ...cmdArgs] = composeCmd(projectName, composeFile, args);
  const proc = spawn(cmd, cmdArgs, {
    cwd: ETHREX_ROOT,
    env: { ...process.env, ...(opts.env || {}) },
    stdio: opts.stdio || "pipe",
  });

  const promise = new Promise((resolve, reject) => {
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

  // Expose the child process so callers can track/kill it
  promise.process = proc;
  return promise;
}

/** Build Docker images for the deployment.
 * @param {object} opts
 * @param {boolean} [opts.forceRebuild=false] - When true, removes existing images before building.
 *   When false (default), reuses existing images if found.
 */
async function buildImages(projectName, composeFile, env = {}, onLog, { forceRebuild = false } = {}) {
  if (forceRebuild) {
    // Remove any existing images for this project to force a clean build
    try {
      const existing = execSync(
        `docker images --filter "reference=tokamak-appchain:*-${projectName}" --format "{{.Repository}}:{{.Tag}}"`,
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
  }
  return runCompose(projectName, composeFile, ["build"], { env, onLog });
}

/** Start L1 service */
async function startL1(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "--no-build", "tokamak-app-l1"], { env });
}

/** Run contract deployer (waits for completion) */
async function deployContracts(projectName, composeFile, env = {}, onLog) {
  return runCompose(projectName, composeFile, ["up", "--no-build", "tokamak-app-deployer"], {
    env,
    onLog,
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

/**
 * Write environment variables to the deployer env volume.
 * Used when addresses are recovered from logs but the .env file wasn't written.
 */
function writeEnvToVolume(projectName, envVars) {
  const volumeName = `${projectName}_env`;
  const tempDir = path.join(require("os").tmpdir(), `ethrex-write-${projectName}`);
  fs.mkdirSync(tempDir, { recursive: true });

  try {
    // Build .env content
    const envContent = Object.entries(envVars)
      .filter(([, v]) => v != null && v !== "")
      .map(([k, v]) => `${k}=${v}`)
      .join("\n") + "\n";

    fs.writeFileSync(path.join(tempDir, ".env"), envContent);
    console.log(`[writeEnvToVolume] Writing to volume ${volumeName}:\n${envContent}`);

    execSync(
      `docker run --rm -v ${volumeName}:/env -v ${tempDir}:/in alpine cp /in/.env /env/.env`,
      { cwd: ETHREX_ROOT, timeout: 30000 }
    );
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

/** Start L2 service (--no-deps: don't restart deployer) */
async function startL2(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "--no-deps", "--no-build", "tokamak-app-l2"], { env });
}

/** Start prover service (--no-deps: don't restart deployer or L2) */
async function startProver(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "--no-deps", "--no-build", "tokamak-app-prover"], { env });
}

/** Stop a single service */
async function stopService(projectName, composeFile, service) {
  return runCompose(projectName, composeFile, ["stop", service], { ignoreError: true });
}

/** Start a single service */
async function startService(projectName, composeFile, service, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", service], { env, ignoreError: true });
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

/** Build base tools environment from port config */
function buildToolsEnv(toolsPorts) {
  const env = {
    TOOLS_L1_EXPLORER_PORT: String(toolsPorts.toolsL1ExplorerPort || 8083),
    TOOLS_L2_EXPLORER_PORT: String(toolsPorts.toolsL2ExplorerPort || 8082),
    TOOLS_BRIDGE_UI_PORT: String(toolsPorts.toolsBridgeUIPort || 3000),
    TOOLS_DB_PORT: String(toolsPorts.toolsDbPort || 7432),
    TOOLS_L1_RPC_PORT: String(toolsPorts.l1Port || 8545),
    TOOLS_L2_RPC_PORT: String(toolsPorts.l2Port || 1729),
    TOOLS_METRICS_PORT: String(toolsPorts.toolsMetricsPort || 3702),
  };
  // External L1 config (testnet/mainnet)
  if (toolsPorts.l1RpcUrl) env.L1_RPC_URL = toolsPorts.l1RpcUrl;
  if (toolsPorts.l1ChainId) env.L1_CHAIN_ID = String(toolsPorts.l1ChainId);
  if (toolsPorts.l1ExplorerUrl) env.L1_EXPLORER_URL = toolsPorts.l1ExplorerUrl;
  if (toolsPorts.l1NetworkName) env.L1_NETWORK_NAME = toolsPorts.l1NetworkName;
  if (toolsPorts.l2ChainId) env.L2_CHAIN_ID = String(toolsPorts.l2ChainId);
  if (toolsPorts.isExternalL1) env.IS_EXTERNAL_L1 = 'true';

  // Always set Blockscout HOST/PROTOCOL defaults (avoids nested variable substitution in compose)
  env.PUBLIC_L2_EXPLORER_HOST = `localhost:${toolsPorts.toolsL2ExplorerPort || 8082}`;
  env.PUBLIC_L2_EXPLORER_PROTOCOL = 'http';
  env.PUBLIC_L2_WS_PROTOCOL = 'ws';
  env.PUBLIC_L1_EXPLORER_HOST = `localhost:${toolsPorts.toolsL1ExplorerPort || 8083}`;
  env.PUBLIC_L1_EXPLORER_PROTOCOL = 'http';
  env.PUBLIC_L1_WS_PROTOCOL = 'ws';

  // Public access config (external domain/IP) — overrides defaults above
  if (toolsPorts.publicDomain) {
    env.PUBLIC_DOMAIN = toolsPorts.publicDomain;
    env.PUBLIC_BASE_URL = toolsPorts.publicBaseUrl || `http://${toolsPorts.publicDomain}`;
    if (toolsPorts.publicL2RpcUrl) env.PUBLIC_L2_RPC_URL = toolsPorts.publicL2RpcUrl;
    if (toolsPorts.publicL2ExplorerUrl) env.PUBLIC_L2_EXPLORER_URL = toolsPorts.publicL2ExplorerUrl;
    if (toolsPorts.publicL1ExplorerUrl) env.PUBLIC_L1_EXPLORER_URL = toolsPorts.publicL1ExplorerUrl;
    if (toolsPorts.publicDashboardUrl) env.PUBLIC_DASHBOARD_URL = toolsPorts.publicDashboardUrl;
    // Blockscout frontend HOST + PROTOCOL (extract from full URL)
    for (const [urlKey, hostKey, protoKey, wsKey] of [
      ['publicL2ExplorerUrl', 'PUBLIC_L2_EXPLORER_HOST', 'PUBLIC_L2_EXPLORER_PROTOCOL', 'PUBLIC_L2_WS_PROTOCOL'],
      ['publicL1ExplorerUrl', 'PUBLIC_L1_EXPLORER_HOST', 'PUBLIC_L1_EXPLORER_PROTOCOL', 'PUBLIC_L1_WS_PROTOCOL'],
    ]) {
      if (toolsPorts[urlKey]) {
        try {
          const u = new URL(toolsPorts[urlKey]);
          env[hostKey] = u.host;
          env[protoKey] = u.protocol.replace(':', '');
          env[wsKey] = u.protocol === 'https:' ? 'wss' : 'ws';
        } catch (e) {
          console.warn(`[docker-local] Failed to parse public URL for ${urlKey}: ${e.message}`);
        }
      }
    }
  }
  return env;
}

/** Build docker compose up args, selecting L2-only services for external L1 */
function buildToolsUpArgs(toolsCompose, toolsPorts, projectName) {
  const upArgs = ["compose", "-f", toolsCompose];
  if (projectName) upArgs.push("-p", projectName);
  if (toolsPorts.skipL1Explorer) {
    upArgs.push("--profile", "external-l1");
    upArgs.push("up", "-d");
    upArgs.push("frontend-l2", "backend-l2", "db", "db-init", "redis-db", "function-selectors-l2", "bridge-ui", "proxy-l2-only");
  } else {
    upArgs.push("up", "-d");
  }
  return upArgs;
}

/** Start support tools (Blockscout, Bridge UI, Dashboard) using the existing tools compose file.
 *  @param {string} projectName - Docker Compose project name for per-deployment isolation (e.g. "tokamak-08cab1ae-tools")
 */
async function startTools(projectName, envVars, toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");

  if (!fs.existsSync(toolsCompose)) {
    throw new Error("Tools compose file not found: " + toolsCompose);
  }

  // Write per-deployment .env file (avoids overwriting other deployments)
  const envFileName = projectName ? `.deployed-${projectName}.env` : ".zk-dex-deployed.env";
  const envPath = path.join(l2Dir, envFileName);
  const envLines = Object.entries(envVars || {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
  fs.writeFileSync(envPath, envLines + "\n");

  const toolsEnv = { ...buildToolsEnv(toolsPorts), TOOLS_ENV_FILE: envPath };

  // Build bridge UI image
  const buildArgs = ["compose", "-f", toolsCompose];
  if (projectName) buildArgs.push("-p", projectName);
  buildArgs.push("build");
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", buildArgs, {
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

  // Start tools (optionally skip L1 explorer for external L1)
  const upArgs = buildToolsUpArgs(toolsCompose, toolsPorts, projectName);
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", upArgs, {
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
async function buildTools(projectName, toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");

  if (!fs.existsSync(toolsCompose)) {
    throw new Error("Tools compose file not found: " + toolsCompose);
  }

  const toolsEnv = buildToolsEnv(toolsPorts);
  const envFile = resolveToolsEnvFile(projectName);
  if (envFile) toolsEnv.TOOLS_ENV_FILE = envFile;

  const buildArgs = ["compose", "-f", toolsCompose];
  if (projectName) buildArgs.push("-p", projectName);
  buildArgs.push("build");
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", buildArgs, {
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
async function restartTools(projectName, envVars, toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");

  if (!fs.existsSync(toolsCompose)) {
    throw new Error("Tools compose file not found: " + toolsCompose);
  }

  // Write per-deployment .env file
  const envFileName = projectName ? `.deployed-${projectName}.env` : ".zk-dex-deployed.env";
  const envPath = path.join(l2Dir, envFileName);
  const envLines = Object.entries(envVars || {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
  fs.writeFileSync(envPath, envLines + "\n");

  const toolsEnv = { ...buildToolsEnv(toolsPorts), TOOLS_ENV_FILE: envPath };

  // Stop existing tools for this deployment only
  const downArgs = ["compose", "-f", toolsCompose];
  if (projectName) downArgs.push("-p", projectName);
  downArgs.push("down", "--remove-orphans");
  await new Promise((resolve) => {
    const proc = spawn("docker", downArgs, {
      cwd: l2Dir,
      env: { ...process.env, ...toolsEnv },
      stdio: "pipe",
    });
    proc.on("close", () => resolve());
    proc.on("error", () => resolve());
  });

  // Start without build
  const restartUpArgs = buildToolsUpArgs(toolsCompose, toolsPorts, projectName);
  await new Promise((resolve, reject) => {
    const proc = spawn("docker", restartUpArgs, {
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
async function getToolsLogs(projectName, service, tail = 100) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  if (!fs.existsSync(toolsCompose)) return "";

  const args = ["compose", "-f", toolsCompose];
  if (projectName) args.push("-p", projectName);
  args.push("logs", "--tail", String(tail));
  if (service) args.push(service);

  const toolsEnvFile = resolveToolsEnvFile(projectName);
  const env = toolsEnvFile ? { ...process.env, TOOLS_ENV_FILE: toolsEnvFile } : undefined;

  return new Promise((resolve) => {
    const proc = spawn("docker", args, { cwd: l2Dir, env, stdio: "pipe" });
    let stdout = "";
    if (proc.stdout) proc.stdout.on("data", (d) => (stdout += d));
    if (proc.stderr) proc.stderr.on("data", (d) => (stdout += d));
    proc.on("close", () => resolve(stdout));
    proc.on("error", () => resolve(""));
  });
}

/** Stream logs for a tools service (returns spawned process) */
function streamToolsLogs(projectName, service) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  const args = ["compose", "-f", toolsCompose];
  if (projectName) args.push("-p", projectName);
  args.push("logs", "-f", "--tail", "50");
  if (service) args.push(service);

  const toolsEnvFile = resolveToolsEnvFile(projectName);
  const env = toolsEnvFile ? { ...process.env, TOOLS_ENV_FILE: toolsEnvFile } : undefined;
  return spawn("docker", args, { cwd: l2Dir, env, stdio: "pipe" });
}

/** Resolve the per-deployment tools env file path for a given project name */
function resolveToolsEnvFile(projectName) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const envFileName = projectName ? `.deployed-${projectName}.env` : ".zk-dex-deployed.env";
  const envPath = path.join(l2Dir, envFileName);
  return fs.existsSync(envPath) ? envPath : null;
}

/** Get support tools container status for a specific deployment */
async function getToolsStatus(projectName) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  if (!fs.existsSync(toolsCompose)) return [];

  const args = ["compose", "-f", toolsCompose];
  if (projectName) args.push("-p", projectName);
  args.push("ps", "--format", "json");

  const toolsEnvFile = resolveToolsEnvFile(projectName);
  const env = toolsEnvFile ? { TOOLS_ENV_FILE: toolsEnvFile } : {};

  try {
    const result = await new Promise((resolve, reject) => {
      const proc = spawn("docker", args, {
        cwd: l2Dir,
        env: { ...process.env, ...env },
        stdio: "pipe",
      });
      let stdout = "";
      proc.stdout.on("data", (d) => (stdout += d));
      proc.on("close", () => resolve(stdout));
      proc.on("error", reject);
    });
    return result
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => { try { return JSON.parse(line); } catch { return null; } })
      .filter(Boolean);
  } catch {
    return [];
  }
}

/** Stop support tools for a specific deployment */
async function stopTools(projectName) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, "docker-compose-zk-dex-tools.yaml");
  if (!fs.existsSync(toolsCompose)) return;

  // Stop all profiles (default + external-l1) to ensure proxy-l2 etc. are also stopped
  const args = ["compose", "-f", toolsCompose];
  if (projectName) args.push("-p", projectName);
  args.push("--profile", "*", "down", "--remove-orphans");

  const toolsEnvFile = resolveToolsEnvFile(projectName);
  const env = toolsEnvFile ? { TOOLS_ENV_FILE: toolsEnvFile } : {};

  await new Promise((resolve) => {
    const proc = spawn("docker", args, {
      cwd: l2Dir,
      env: { ...process.env, ...env },
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

/** Check if NVIDIA GPU is available via nvidia-smi */
function hasNvidiaGpu() {
  try {
    execSync("nvidia-smi", { stdio: "ignore", timeout: 5000 });
    return true;
  } catch {
    return false;
  }
}

/** Find an existing Docker image for a programSlug (e.g. evm-l2, zk-dex) */
function findImage(programSlug) {
  // Sanitize slug: only allow alphanumeric, hyphens, and underscores to prevent command injection
  if (!programSlug || !/^[a-zA-Z0-9_-]+$/.test(programSlug)) {
    return null;
  }
  try {
    // First check shared name: tokamak-appchain:{slug}
    const shared = execSync(`docker image inspect "tokamak-appchain:${programSlug}" --format "{{.Id}}"`, { timeout: 10000, stdio: ['pipe', 'pipe', 'pipe'] });
    if (shared.toString().trim()) return `tokamak-appchain:${programSlug}`;
  } catch {}
  try {
    // Then check any project-specific name: tokamak-appchain:{slug}-*
    const result = execSync(`docker images --filter "reference=tokamak-appchain:${programSlug}-*" --format "{{.Repository}}:{{.Tag}}"`, { timeout: 10000 });
    const first = result.toString().trim().split("\n").filter(Boolean)[0];
    if (first) return first;
  } catch {}
  return null;
}

module.exports = {
  findImage,
  buildImages,
  startL1,
  deployContracts,
  extractEnv,
  writeEnvToVolume,
  stopService,
  startService,
  startL2,
  startProver,
  stop,
  start,
  destroy,
  getStatus,
  getLogs,
  streamLogs,
  isDockerAvailable,
  hasNvidiaGpu,
  startTools,
  buildTools,
  restartTools,
  getToolsLogs,
  streamToolsLogs,
  stopTools,
  getToolsStatus,
  ETHREX_ROOT,
};

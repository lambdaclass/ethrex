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
    if (proc.stdout) proc.stdout.on("data", (d) => (stdout += d));
    if (proc.stderr) proc.stderr.on("data", (d) => (stderr += d));

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
async function buildImages(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["build"], { env });
}

/** Pull pre-built Docker images from registry */
async function pullImages(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["pull"], { env, timeout: 300000 });
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
    const parsed = {};
    for (const line of envContent.split("\n")) {
      const match = line.match(/^([^=]+)=(.*)$/);
      if (match) parsed[match[1].trim()] = match[2].trim();
    }
    return parsed;
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

/** Start L2 service */
async function startL2(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "tokamak-app-l2"], { env });
}

/** Start prover service */
async function startProver(projectName, composeFile, env = {}) {
  return runCompose(projectName, composeFile, ["up", "-d", "tokamak-app-prover"], { env });
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

/** Check if a local Docker image exists */
function imageExists(imageName) {
  try {
    const result = execSync(`docker image inspect ${imageName}`, {
      stdio: "pipe",
      timeout: 5000,
    });
    return true;
  } catch {
    return false;
  }
}

/** Build platform images using platform/Dockerfile (multi-target) */
async function buildPlatformImages() {
  const platformDir = path.resolve(__dirname, "../..");
  const dockerfile = path.join(platformDir, "Dockerfile");

  // Build L1 and L2 images sequentially
  for (const { target, tag } of [
    { target: "l1", tag: "tokamak-app-l1:latest" },
    { target: "l2", tag: "tokamak-app-l2:latest" },
  ]) {
    await new Promise((resolve, reject) => {
      const proc = spawn("docker", [
        "build", "-f", dockerfile, "--target", target, "-t", tag, ETHREX_ROOT,
      ], {
        cwd: ETHREX_ROOT,
        stdio: "pipe",
      });

      let stderr = "";
      if (proc.stderr) proc.stderr.on("data", (d) => (stderr += d));

      proc.on("close", (code) => {
        if (code !== 0) {
          reject(new Error(`docker build ${target} failed (code ${code}): ${stderr.slice(-500)}`));
        } else {
          resolve();
        }
      });
      proc.on("error", reject);
    });
  }
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
  pullImages,
  buildPlatformImages,
  imageExists,
  startL1,
  deployContracts,
  extractEnv,
  startL2,
  startProver,
  stop,
  start,
  destroy,
  getStatus,
  getLogs,
  streamLogs,
  isDockerAvailable,
  ETHREX_ROOT,
};

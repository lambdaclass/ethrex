/**
 * Remote Docker engine — deploys L2 via SSH to a remote server.
 *
 * Strategy: pre-built Docker images (no source code needed on remote).
 * 1. SSH connect to the remote host
 * 2. Upload docker-compose.yaml + config files via SFTP
 * 3. `docker compose pull` (pulls pre-built images)
 * 4. `docker compose up -d` (starts services)
 * 5. Stream logs / get status via SSH
 */

const { Client } = require("ssh2");

/**
 * Create an SSH connection to a remote host.
 * @param {Object} host - Host record from DB
 * @returns {Promise<Client>} Connected SSH client
 */
function connect(host) {
  return new Promise((resolve, reject) => {
    const conn = new Client();
    const config = {
      host: host.hostname,
      port: host.port || 22,
      username: host.username,
      readyTimeout: 10000,
    };

    if (host.auth_method === "key" && host.private_key) {
      config.privateKey = host.private_key;
    }

    conn.on("ready", () => resolve(conn));
    conn.on("error", (err) => reject(new Error(`SSH connection failed: ${err.message}`)));
    conn.connect(config);
  });
}

/**
 * Execute a command on the remote host.
 * @returns {Promise<{stdout: string, stderr: string, code: number}>}
 */
function exec(conn, command, opts = {}) {
  return new Promise((resolve, reject) => {
    conn.exec(command, (err, stream) => {
      if (err) return reject(err);
      let stdout = "";
      let stderr = "";
      let settled = false;

      if (opts.timeout) {
        const timer = setTimeout(() => {
          if (!settled) { settled = true; try { stream.close(); } catch {} reject(new Error("Remote command timed out")); }
        }, opts.timeout);
        stream.on("close", () => clearTimeout(timer));
      }

      stream.on("close", (code) => {
        if (settled) return;
        settled = true;
        if (code !== 0 && !opts.ignoreError) {
          reject(new Error(`Remote command failed (code ${code}): ${stderr}`));
        } else {
          resolve({ stdout, stderr, code });
        }
      });
      stream.on("data", (data) => (stdout += data));
      stream.stderr.on("data", (data) => (stderr += data));
    });
  });
}

/**
 * Upload a file to the remote host via SFTP.
 */
function uploadFile(conn, localContent, remotePath) {
  return new Promise((resolve, reject) => {
    conn.sftp((err, sftp) => {
      if (err) return reject(err);
      const stream = sftp.createWriteStream(remotePath);
      stream.on("close", () => resolve());
      stream.on("error", (err) => reject(err));
      stream.end(localContent);
    });
  });
}

/**
 * Upload compose file and configs, then pull + start services.
 */
/** Validate shell-safe name (alphanumeric, hyphens, underscores only) */
function assertSafeName(value, label) {
  if (!/^[a-zA-Z0-9._-]+$/.test(value)) {
    throw new Error(`Invalid ${label}: must be alphanumeric/hyphens/underscores only`);
  }
}

/** Shell-quote a path value */
function q(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

async function deployRemote(conn, projectName, composeContent, remoteDir) {
  assertSafeName(projectName, "projectName");

  // Create deployment directory
  await exec(conn, `mkdir -p ${q(remoteDir)}`);

  // Upload compose file
  await uploadFile(conn, composeContent, `${remoteDir}/docker-compose.yaml`);

  // Pull images (pre-built, no build needed)
  await exec(conn, `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} pull`, {
    timeout: 300000, // 5 min to pull images
  });

  // Start services
  await exec(conn, `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} up -d`, {
    timeout: 60000,
  });
}

/**
 * Extract .env from deployer volume on remote.
 */
async function extractEnvRemote(conn, projectName) {
  assertSafeName(projectName, "projectName");
  const { stdout } = await exec(
    conn,
    `docker run --rm -v ${q(projectName + '_env')}:/env alpine cat /env/.env`,
    { ignoreError: true, timeout: 30000 }
  );

  const parsed = {};
  for (const line of stdout.split("\n")) {
    const match = line.match(/^([^=]+)=(.*)$/);
    if (match) parsed[match[1].trim()] = match[2].trim();
  }
  return parsed;
}

/** Stop services on remote */
async function stopRemote(conn, projectName, remoteDir) {
  await exec(conn, `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} stop`, {
    ignoreError: true,
    timeout: 60000,
  });
}

/** Start stopped services on remote */
async function startRemote(conn, projectName, remoteDir) {
  // Use .keys.env if it exists (testnet deployments)
  const { stdout } = await exec(conn, `test -f ${q(remoteDir + '/.keys.env')} && echo yes || echo no`, { ignoreError: true });
  const envFileFlag = stdout.trim() === "yes" ? `--env-file ${q(remoteDir + '/.keys.env')}` : "";
  await exec(conn, `cd ${q(remoteDir)} && docker compose ${envFileFlag} -p ${q(projectName)} up -d`, {
    timeout: 60000,
  });
}

/** Destroy services + volumes on remote */
async function destroyRemote(conn, projectName, remoteDir) {
  await exec(
    conn,
    `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} down --volumes --remove-orphans`,
    { ignoreError: true, timeout: 60000 }
  );
  await exec(conn, `rm -rf ${q(remoteDir)}`, { ignoreError: true });
}

/** Get container status on remote */
async function getStatusRemote(conn, projectName, remoteDir) {
  try {
    const { stdout } = await exec(
      conn,
      `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} ps --format json`,
      { ignoreError: true, timeout: 15000 }
    );
    return stdout
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        try { return JSON.parse(line); } catch { return null; }
      })
      .filter(Boolean);
  } catch {
    return [];
  }
}

/** Get logs from remote */
async function getLogsRemote(conn, projectName, remoteDir, service, tail = 100) {
  const svc = service ? ` ${service}` : "";
  const { stdout } = await exec(
    conn,
    `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} logs --tail ${tail}${svc}`,
    { ignoreError: true, timeout: 15000 }
  );
  return stdout;
}

/** Stream logs from remote (returns the SSH stream) */
function streamLogsRemote(conn, projectName, remoteDir, service) {
  return new Promise((resolve, reject) => {
    const svc = service ? ` ${service}` : "";
    conn.exec(
      `cd ${q(remoteDir)} && docker compose -p ${q(projectName)} logs -f --tail 50${svc}`,
      (err, stream) => {
        if (err) return reject(err);
        resolve(stream);
      }
    );
  });
}

/**
 * Test SSH connection to a host.
 * @returns {Promise<{ok: boolean, docker: boolean, message: string}>}
 */
async function testConnection(host) {
  let conn;
  try {
    conn = await connect(host);

    // Test basic connectivity
    const { stdout: hostname } = await exec(conn, "hostname");

    // Test Docker availability
    let dockerOk = false;
    try {
      await exec(conn, "docker info --format '{{.ServerVersion}}'");
      dockerOk = true;
    } catch {
      // Docker not available
    }

    // Test Docker Compose
    let composeOk = false;
    if (dockerOk) {
      try {
        await exec(conn, "docker compose version --short");
        composeOk = true;
      } catch {
        // Docker Compose not available
      }
    }

    conn.end();
    return {
      ok: true,
      docker: dockerOk && composeOk,
      message: dockerOk && composeOk
        ? `Connected to ${hostname.trim()}. Docker + Compose available.`
        : `Connected to ${hostname.trim()}. ${!dockerOk ? "Docker not found." : "Docker Compose not found."}`,
    };
  } catch (err) {
    if (conn) conn.end();
    return { ok: false, docker: false, message: err.message };
  }
}

/**
 * Deploy and start tools (Blockscout, Bridge UI, Dashboard) on a remote server.
 *
 * Strategy: upload the tools compose file + env, then docker compose up.
 * Bridge UI is built from a Dockerfile, so we need the tooling/bridge context
 * available on the remote. Instead, we use a pre-built bridge-ui image or
 * build it on the remote from a minimal context.
 *
 * @param {Client} conn - SSH connection
 * @param {string} projectName - Tools project name (e.g. "tokamak-12345678-tools")
 * @param {string} remoteDir - Remote directory for the deployment
 * @param {Object} envVars - Deployed contract addresses from deployer
 * @param {Object} toolsPorts - Port configuration from getToolsPorts()
 * @param {Object} [opts] - Optional settings
 * @param {boolean} [opts.skipL1Explorer] - Skip L1 Blockscout (external L1)
 */
async function startToolsRemote(conn, projectName, remoteDir, envVars, toolsPorts, opts = {}) {
  assertSafeName(projectName, "projectName");
  const toolsDir = `${remoteDir}/tools`;
  await exec(conn, `mkdir -p ${q(toolsDir)}`);

  // Write the deployed .env file for tools (contract addresses etc.)
  const envContent = Object.entries(envVars || {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n") + "\n";
  await uploadFile(conn, envContent, `${toolsDir}/deployed.env`);

  // Build tools environment variables
  const toolsEnv = {
    TOOLS_L1_EXPLORER_PORT: String(toolsPorts.toolsL1ExplorerPort || 8083),
    TOOLS_L2_EXPLORER_PORT: String(toolsPorts.toolsL2ExplorerPort || 8082),
    TOOLS_BRIDGE_UI_PORT: String(toolsPorts.toolsBridgeUIPort || 3000),
    TOOLS_DB_PORT: String(toolsPorts.toolsDbPort || 7432),
    TOOLS_L1_RPC_PORT: String(toolsPorts.l1Port || 8545),
    TOOLS_L2_RPC_PORT: String(toolsPorts.l2Port || 1729),
    TOOLS_METRICS_PORT: String(toolsPorts.toolsMetricsPort || 3702),
    TOOLS_BIND_ADDR: "0.0.0.0",
    TOOLS_ENV_FILE: `${toolsDir}/deployed.env`,
  };
  if (toolsPorts.l2ChainId) toolsEnv.L2_CHAIN_ID = String(toolsPorts.l2ChainId);
  if (toolsPorts.l1ChainId) toolsEnv.L1_CHAIN_ID = String(toolsPorts.l1ChainId);
  if (toolsPorts.l1RpcUrl) toolsEnv.L1_RPC_URL = toolsPorts.l1RpcUrl;
  if (toolsPorts.l1NetworkName) toolsEnv.L1_NETWORK_NAME = toolsPorts.l1NetworkName;
  if (toolsPorts.isExternalL1) toolsEnv.IS_EXTERNAL_L1 = "true";

  // Blockscout HOST defaults (remote = use hostname:port)
  const hostname = toolsPorts.hostname || "localhost";
  toolsEnv.PUBLIC_L2_EXPLORER_HOST = `${hostname}:${toolsPorts.toolsL2ExplorerPort || 8082}`;
  toolsEnv.PUBLIC_L2_EXPLORER_PROTOCOL = "http";
  toolsEnv.PUBLIC_L2_WS_PROTOCOL = "ws";
  toolsEnv.PUBLIC_L1_EXPLORER_HOST = `${hostname}:${toolsPorts.toolsL1ExplorerPort || 8083}`;
  toolsEnv.PUBLIC_L1_EXPLORER_PROTOCOL = "http";
  toolsEnv.PUBLIC_L1_WS_PROTOCOL = "ws";

  // Download tools compose file from the container's ethrex source
  // The tools compose references a build context (tooling/bridge) which isn't available remotely.
  // We download it from the repository and use it directly.
  // For bridge-ui, we'll clone just the needed files.
  const toolsComposeUrl = "https://raw.githubusercontent.com/tokamak-network/ethrex/tokamak-dev/crates/l2/docker-compose-zk-dex-tools.yaml";
  await exec(conn, `curl -fsSL "${toolsComposeUrl}" -o ${toolsDir}/docker-compose-tools.yaml`, {
    timeout: 30000,
  });

  // Clone the bridge tooling directory for the build context
  await exec(conn, `
    if [ ! -d ${toolsDir}/tooling ]; then
      git clone --depth=1 --filter=blob:none --sparse https://github.com/tokamak-network/ethrex.git ${toolsDir}/ethrex-sparse 2>/dev/null || true
      cd ${toolsDir}/ethrex-sparse && git sparse-checkout set crates/l2/tooling/bridge 2>/dev/null || true
      cp -r ${toolsDir}/ethrex-sparse/crates/l2/tooling ${toolsDir}/tooling 2>/dev/null || true
      rm -rf ${toolsDir}/ethrex-sparse
    fi
  `.trim(), { timeout: 60000, ignoreError: true });

  // Fix the tools compose build context to use our cloned path
  await exec(conn, `sed -i 's|context: ./tooling/bridge|context: ${toolsDir}/tooling/bridge|g' ${toolsDir}/docker-compose-tools.yaml`, {
    ignoreError: true,
  });

  // Write env file on remote (safer than shell export for values with special chars)
  const envLines = Object.entries(toolsEnv).map(([k, v]) => `${k}='${String(v).replace(/'/g, "'\\''")}'`).join("\n");
  await exec(conn, `cat > ${toolsDir}/.tools.env << 'ENVEOF'\n${envLines}\nENVEOF`);
  const envFileFlag = `--env-file ${toolsDir}/.tools.env`;

  // Build + start tools
  const profile = opts.skipL1Explorer ? "--profile external-l1" : "";
  const services = opts.skipL1Explorer
    ? "frontend-l2 backend-l2 db db-init redis-db function-selectors-l2 bridge-ui proxy-l2-only"
    : "";

  await exec(conn, `cd ${q(toolsDir)} && docker compose ${envFileFlag} -f docker-compose-tools.yaml -p ${q(projectName)} ${profile} build`, {
    timeout: 300000,
  });

  await exec(conn, `cd ${q(toolsDir)} && docker compose ${envFileFlag} -f docker-compose-tools.yaml -p ${q(projectName)} ${profile} up -d ${services}`, {
    timeout: 120000,
  });
}

/** Stop tools on remote */
async function stopToolsRemote(conn, projectName, remoteDir) {
  const toolsDir = `${remoteDir}/tools`;
  await exec(conn, `cd ${q(toolsDir)} && docker compose -f docker-compose-tools.yaml -p ${q(projectName)} stop`, {
    ignoreError: true, timeout: 60000,
  });
}

module.exports = {
  connect,
  exec,
  uploadFile,
  deployRemote,
  extractEnvRemote,
  stopRemote,
  startRemote,
  destroyRemote,
  getStatusRemote,
  getLogsRemote,
  streamLogsRemote,
  testConnection,
  startToolsRemote,
  stopToolsRemote,
};

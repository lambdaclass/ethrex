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
      stream.on("close", (code) => {
        if (code !== 0 && !opts.ignoreError) {
          reject(new Error(`Remote command failed (code ${code}): ${stderr}`));
        } else {
          resolve({ stdout, stderr, code });
        }
      });
      stream.on("data", (data) => (stdout += data));
      stream.stderr.on("data", (data) => (stderr += data));
    });

    if (opts.timeout) {
      setTimeout(() => reject(new Error("Remote command timed out")), opts.timeout);
    }
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
async function deployRemote(conn, projectName, composeContent, remoteDir) {
  // Create deployment directory
  await exec(conn, `mkdir -p ${remoteDir}`);

  // Upload compose file
  await uploadFile(conn, composeContent, `${remoteDir}/docker-compose.yaml`);

  // Pull images (pre-built, no build needed)
  await exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} pull`, {
    timeout: 300000, // 5 min to pull images
  });

  // Start services
  await exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d`, {
    timeout: 60000,
  });
}

/**
 * Extract .env from deployer volume on remote.
 */
async function extractEnvRemote(conn, projectName) {
  const { stdout } = await exec(
    conn,
    `docker run --rm -v ${projectName}_env:/env alpine cat /env/.env`,
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
  await exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} stop`, {
    ignoreError: true,
    timeout: 60000,
  });
}

/** Start stopped services on remote */
async function startRemote(conn, projectName, remoteDir) {
  await exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d`, {
    timeout: 60000,
  });
}

/** Destroy services + volumes on remote */
async function destroyRemote(conn, projectName, remoteDir) {
  await exec(
    conn,
    `cd ${remoteDir} && docker compose -p ${projectName} down --volumes --remove-orphans`,
    { ignoreError: true, timeout: 60000 }
  );
  await exec(conn, `rm -rf ${remoteDir}`, { ignoreError: true });
}

/** Get container status on remote */
async function getStatusRemote(conn, projectName, remoteDir) {
  try {
    const { stdout } = await exec(
      conn,
      `cd ${remoteDir} && docker compose -p ${projectName} ps --format json`,
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
    `cd ${remoteDir} && docker compose -p ${projectName} logs --tail ${tail}${svc}`,
    { ignoreError: true, timeout: 15000 }
  );
  return stdout;
}

/** Stream logs from remote (returns the SSH stream) */
function streamLogsRemote(conn, projectName, remoteDir, service) {
  return new Promise((resolve, reject) => {
    const svc = service ? ` ${service}` : "";
    conn.exec(
      `cd ${remoteDir} && docker compose -p ${projectName} logs -f --tail 50${svc}`,
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
};

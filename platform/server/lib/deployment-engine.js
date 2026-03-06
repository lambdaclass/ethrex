/**
 * Deployment Engine — orchestrates the full L2 deployment lifecycle.
 *
 * Supports two modes:
 * - Local: builds from source via Docker Compose on the platform host
 * - Remote: uses pre-built images, deploys via SSH to a remote server
 *
 * State machine: configured → building/pulling → l1_starting → deploying_contracts → l2_starting → running
 * On error: → error (with rollback)
 */

const EventEmitter = require("events");
const docker = require("./docker-local");
const remote = require("./docker-remote");
const {
  generateComposeFile,
  generateRemoteComposeFile,
  generateProgramsToml,
  writeComposeFile,
  getDeploymentDir,
  getAppProfile,
} = require("./compose-generator");
const { isHealthy } = require("./rpc-client");
const { updateDeployment, getNextAvailablePorts } = require("../db/deployments");
const { getHostById } = require("../db/hosts");

// Active deployments event emitters (keyed by deployment ID)
const deploymentEvents = new Map();

const PHASES = [
  "configured",
  "checking_docker",
  "building",
  "pulling",
  "l1_starting",
  "deploying_contracts",
  "l2_starting",
  "starting_prover",
  "starting_tools",
  "running",
];

function getEmitter(deploymentId) {
  if (!deploymentEvents.has(deploymentId)) {
    deploymentEvents.set(deploymentId, new EventEmitter());
  }
  return deploymentEvents.get(deploymentId);
}

function emit(deploymentId, event, data) {
  const emitter = deploymentEvents.get(deploymentId);
  if (emitter) {
    emitter.emit("event", { event, ...data, timestamp: Date.now() });
  }
}

// ============================================================
// LOCAL PROVISIONING (build from source)
// ============================================================

async function provision(deployment) {
  const { id, program_slug: programSlug } = deployment;

  emit(id, "phase", { phase: "checking_docker", message: "Checking Docker availability..." });
  updateDeployment(id, { phase: "checking_docker", error_message: null });

  if (!docker.isDockerAvailable()) {
    const errMsg = "Docker is not running. Please install and start Docker Desktop first.";
    emit(id, "error", { message: errMsg });
    updateDeployment(id, { phase: "error", error_message: errMsg });
    throw new Error(errMsg);
  }

  emit(id, "phase", { phase: "checking_docker", message: "Docker is available" });

  // Parse config
  let deployDir = null;
  try {
    const config = deployment.config ? JSON.parse(deployment.config) : {};
    deployDir = config.deployDir || null;
  } catch {}

  const { l1Port, l2Port, proofCoordPort, toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, toolsMetricsPort } = getNextAvailablePorts();
  const projectName = `tokamak-${id.slice(0, 8)}`;

  updateDeployment(id, {
    docker_project: projectName,
    l1_port: l1Port,
    l2_port: l2Port,
    proof_coord_port: proofCoordPort,
    tools_l1_explorer_port: toolsL1ExplorerPort,
    tools_l2_explorer_port: toolsL2ExplorerPort,
    tools_bridge_ui_port: toolsBridgeUIPort,
    tools_db_port: toolsDbPort,
    tools_metrics_port: toolsMetricsPort,
    deploy_dir: deployDir,
    phase: "building",
    error_message: null,
  });

  emit(id, "phase", { phase: "building", message: "Generating Docker Compose configuration..." });

  try {
    const gpu = docker.hasNvidiaGpu();
    const composeContent = generateComposeFile({ programSlug, l1Port, l2Port, proofCoordPort, metricsPort: toolsMetricsPort, projectName, gpu });
    const composeFile = writeComposeFile(id, composeContent, deployDir);

    emit(id, "phase", { phase: "building", message: "Building Docker images... (this may take several minutes on first run)" });
    await docker.buildImages(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" }, (chunk) => {
      // Stream build log lines to SSE
      const lines = chunk.split("\n").filter(Boolean);
      for (const line of lines) {
        emit(id, "log", { message: line });
      }
    });

    emit(id, "phase", { phase: "l1_starting", message: "Starting L1 node..." });
    updateDeployment(id, { phase: "l1_starting" });
    await docker.startL1(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
    await waitForHealthy(`http://127.0.0.1:${l1Port}`, 60000, id);
    emit(id, "phase", { phase: "l1_starting", message: "L1 node is running" });

    emit(id, "phase", { phase: "deploying_contracts", message: "Deploying L1 contracts (bridge, proposer, verifier)..." });
    updateDeployment(id, { phase: "deploying_contracts" });
    await docker.deployContracts(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });

    // Stop the deployer to prevent restart: on-failure from overwriting /env/.env
    await docker.stopService(projectName, composeFile, "tokamak-app-deployer");

    let envVars = {};
    try {
      envVars = await docker.extractEnv(projectName, composeFile);
    } catch (extractErr) {
      emit(id, "log", { message: `Warning: extractEnv failed: ${extractErr.message}, retrying...` });
      await new Promise(r => setTimeout(r, 3000));
      try {
        envVars = await docker.extractEnv(projectName, composeFile);
      } catch (retryErr) {
        emit(id, "log", { message: `extractEnv retry failed: ${retryErr.message}` });
      }
    }

    const bridgeAddress = envVars.ETHREX_WATCHER_BRIDGE_ADDRESS || null;
    const proposerAddress = envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null;
    console.log(`[deployment-engine] extractEnv for ${projectName}: bridge=${bridgeAddress}, proposer=${proposerAddress}`);
    emit(id, "log", { message: `extractEnv [${projectName}]: bridge=${bridgeAddress}, proposer=${proposerAddress}` });

    if (!bridgeAddress || !proposerAddress) {
      throw new Error(
        `Contract deployment incomplete: bridge=${bridgeAddress}, proposer=${proposerAddress}. ` +
        "The deployer may have exited before writing contract addresses."
      );
    }

    // Save with project ID and timestamp for consistency verification
    updateDeployment(id, {
      bridge_address: bridgeAddress,
      proposer_address: proposerAddress,
      env_project_id: projectName,
      env_updated_at: Date.now(),
    });

    emit(id, "phase", { phase: "deploying_contracts", message: "Contracts deployed", bridgeAddress, proposerAddress });

    emit(id, "phase", { phase: "l2_starting", message: "Starting L2 node..." });
    updateDeployment(id, { phase: "l2_starting" });
    await docker.startL2(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
    await waitForHealthy(`http://127.0.0.1:${l2Port}`, 120000, id);
    emit(id, "phase", { phase: "l2_starting", message: "L2 node is running" });

    emit(id, "phase", { phase: "starting_prover", message: "Starting prover..." });
    updateDeployment(id, { phase: "starting_prover" });
    await docker.startProver(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });

    // Start support tools (Blockscout, Bridge UI, Dashboard)
    // Re-read env volume to get the definitive addresses (L2 watcher uses these)
    emit(id, "phase", { phase: "starting_tools", message: "Starting support tools (Blockscout, Bridge UI, Dashboard)..." });
    updateDeployment(id, { phase: "starting_tools" });
    try {
      const freshEnv = await docker.extractEnv(projectName, composeFile);
      const freshBridge = freshEnv.ETHREX_WATCHER_BRIDGE_ADDRESS || null;
      const freshProposer = freshEnv.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null;
      if (freshBridge && freshBridge !== bridgeAddress) {
        console.log(`[deployment-engine] Bridge address changed after L2 start: ${bridgeAddress} -> ${freshBridge}`);
        updateDeployment(id, {
          bridge_address: freshBridge,
          proposer_address: freshProposer,
          env_project_id: projectName,
          env_updated_at: Date.now(),
        });
        envVars = freshEnv;
      }
      await docker.startTools(envVars, { toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, l1Port, l2Port, toolsMetricsPort });
      emit(id, "phase", { phase: "starting_tools", message: "Support tools started" });
    } catch (toolsErr) {
      // Tools failure is non-fatal — deployment still works without them
      emit(id, "phase", { phase: "starting_tools", message: `Tools setup skipped: ${toolsErr.message}` });
    }

    emit(id, "phase", {
      phase: "running",
      message: "Deployment is running!",
      l1Rpc: `http://127.0.0.1:${l1Port}`,
      l2Rpc: `http://127.0.0.1:${l2Port}`,
      bridgeAddress,
      proposerAddress,
    });
    updateDeployment(id, { phase: "running", status: "active" });
    return updateDeployment(id, {});
  } catch (err) {
    emit(id, "error", { message: err.message });
    updateDeployment(id, { phase: "error", error_message: err.message });
    try {
      const composeFile = require("path").join(getDeploymentDir(id), "docker-compose.yaml");
      await docker.destroy(projectName, composeFile);
    } catch {}
    throw err;
  }
}

// ============================================================
// REMOTE PROVISIONING (pre-built images via SSH)
// ============================================================

async function provisionRemote(deployment, hostId) {
  const { id, program_slug: programSlug } = deployment;
  const host = getHostById(hostId);
  if (!host) throw new Error("Host not found");

  const { l1Port, l2Port, proofCoordPort } = getNextAvailablePorts();
  const projectName = `tokamak-${id.slice(0, 8)}`;
  const remoteDir = `/opt/tokamak/${id}`;

  updateDeployment(id, {
    host_id: hostId,
    docker_project: projectName,
    l1_port: l1Port,
    l2_port: l2Port,
    proof_coord_port: proofCoordPort,
    phase: "pulling",
    error_message: null,
  });

  emit(id, "phase", { phase: "pulling", message: `Connecting to ${host.hostname}...` });

  let conn;
  try {
    conn = await remote.connect(host);

    // Generate compose file for remote (pre-built images)
    const composeContent = generateRemoteComposeFile({
      programSlug,
      l1Port,
      l2Port,
      proofCoordPort,
      projectName,
      dataDir: remoteDir,
    });

    // Save locally too
    writeComposeFile(id, composeContent);

    emit(id, "phase", { phase: "pulling", message: "Uploading configuration and pulling images..." });

    // Create remote dir
    await remote.exec(conn, `mkdir -p ${remoteDir}`);

    // Upload compose file
    await remote.uploadFile(conn, composeContent, `${remoteDir}/docker-compose.yaml`);

    // Upload programs.toml if needed
    const profile = getAppProfile(programSlug);
    if (profile.programsToml) {
      const tomlContent = generateProgramsToml(programSlug);
      await remote.uploadFile(conn, tomlContent, `${remoteDir}/programs.toml`);
    }

    // Pull images
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} pull`, {
      timeout: 300000,
    });

    // Start L1
    emit(id, "phase", { phase: "l1_starting", message: "Starting L1 node on remote server..." });
    updateDeployment(id, { phase: "l1_starting" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d tokamak-app-l1`, {
      timeout: 60000,
    });

    // Wait for L1 (via remote curl)
    await waitForRemoteHealthy(conn, l1Port, 60000, id);
    emit(id, "phase", { phase: "l1_starting", message: "L1 node is running" });

    // Deploy contracts
    emit(id, "phase", { phase: "deploying_contracts", message: "Deploying contracts on remote..." });
    updateDeployment(id, { phase: "deploying_contracts" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up tokamak-app-deployer`, {
      timeout: 600000,
    });

    // Extract contract addresses
    let envVars = {};
    try { envVars = await remote.extractEnvRemote(conn, projectName); } catch {}
    const bridgeAddress = envVars.ETHREX_WATCHER_BRIDGE_ADDRESS || null;
    const proposerAddress = envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null;

    if (!bridgeAddress || !proposerAddress) {
      throw new Error(
        `Contract deployment incomplete: bridge=${bridgeAddress}, proposer=${proposerAddress}. ` +
        "The deployer may have exited before writing contract addresses."
      );
    }

    updateDeployment(id, { bridge_address: bridgeAddress, proposer_address: proposerAddress });
    emit(id, "phase", { phase: "deploying_contracts", message: "Contracts deployed", bridgeAddress, proposerAddress });

    // Start L2
    emit(id, "phase", { phase: "l2_starting", message: "Starting L2 node on remote..." });
    updateDeployment(id, { phase: "l2_starting" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d tokamak-app-l2`, {
      timeout: 60000,
    });
    await waitForRemoteHealthy(conn, l2Port, 120000, id);
    emit(id, "phase", { phase: "l2_starting", message: "L2 node is running" });

    // Start prover
    emit(id, "phase", { phase: "starting_prover", message: "Starting prover on remote..." });
    updateDeployment(id, { phase: "starting_prover" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d tokamak-app-prover`, {
      timeout: 60000,
    });

    // Done
    const l1Rpc = `http://${host.hostname}:${l1Port}`;
    const l2Rpc = `http://${host.hostname}:${l2Port}`;
    emit(id, "phase", {
      phase: "running",
      message: `Deployment running on ${host.hostname}!`,
      l1Rpc,
      l2Rpc,
      bridgeAddress,
      proposerAddress,
    });
    updateDeployment(id, { phase: "running", status: "active" });
    conn.end();
    return updateDeployment(id, {});
  } catch (err) {
    emit(id, "error", { message: err.message });
    updateDeployment(id, { phase: "error", error_message: err.message });
    // Cleanup on remote
    if (conn) {
      try {
        await remote.destroyRemote(conn, projectName, remoteDir);
      } catch {}
      conn.end();
    }
    throw err;
  }
}

// ============================================================
// SHARED LIFECYCLE (local + remote)
// ============================================================

async function stopDeployment(deployment) {
  if (deployment.host_id) {
    return await stopDeploymentRemote(deployment);
  }
  const composeFile = require("path").join(getDeploymentDir(deployment.id), "docker-compose.yaml");
  try { await docker.stopTools(); } catch {}
  await docker.stop(deployment.docker_project, composeFile);
  return updateDeployment(deployment.id, { phase: "stopped", status: "configured" });
}

async function startDeployment(deployment) {
  if (deployment.host_id) {
    return await startDeploymentRemote(deployment);
  }
  const composeFile = require("path").join(getDeploymentDir(deployment.id), "docker-compose.yaml");
  await docker.start(deployment.docker_project, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
  return updateDeployment(deployment.id, { phase: "running", status: "active" });
}

async function destroyDeployment(deployment) {
  if (deployment.host_id) {
    return await destroyDeploymentRemote(deployment);
  }
  const composeFile = require("path").join(getDeploymentDir(deployment.id), "docker-compose.yaml");
  try { await docker.stopTools(); } catch {}
  await docker.destroy(deployment.docker_project, composeFile);
  const fs = require("fs");
  const deployDir = getDeploymentDir(deployment.id);
  if (fs.existsSync(deployDir)) fs.rmSync(deployDir, { recursive: true, force: true });
  return updateDeployment(deployment.id, {
    phase: "configured", status: "configured",
    docker_project: null, l1_port: null, l2_port: null, proof_coord_port: null,
    bridge_address: null, proposer_address: null, error_message: null, host_id: null,
    tools_l1_explorer_port: null, tools_l2_explorer_port: null,
    tools_bridge_ui_port: null, tools_db_port: null, tools_metrics_port: null,
    env_project_id: null, env_updated_at: null,
  });
}

// Remote lifecycle helpers
async function stopDeploymentRemote(deployment) {
  const host = getHostById(deployment.host_id);
  const conn = await remote.connect(host);
  const remoteDir = `/opt/tokamak/${deployment.id}`;
  await remote.stopRemote(conn, deployment.docker_project, remoteDir);
  conn.end();
  return updateDeployment(deployment.id, { phase: "stopped", status: "configured" });
}

async function startDeploymentRemote(deployment) {
  const host = getHostById(deployment.host_id);
  const conn = await remote.connect(host);
  const remoteDir = `/opt/tokamak/${deployment.id}`;
  await remote.startRemote(conn, deployment.docker_project, remoteDir);
  conn.end();
  return updateDeployment(deployment.id, { phase: "running", status: "active" });
}

async function destroyDeploymentRemote(deployment) {
  const host = getHostById(deployment.host_id);
  const conn = await remote.connect(host);
  const remoteDir = `/opt/tokamak/${deployment.id}`;
  await remote.destroyRemote(conn, deployment.docker_project, remoteDir);
  conn.end();
  const fs = require("fs");
  const deployDir = getDeploymentDir(deployment.id);
  if (fs.existsSync(deployDir)) fs.rmSync(deployDir, { recursive: true, force: true });
  return updateDeployment(deployment.id, {
    phase: "configured", status: "configured",
    docker_project: null, l1_port: null, l2_port: null, proof_coord_port: null,
    bridge_address: null, proposer_address: null, error_message: null, host_id: null,
    tools_l1_explorer_port: null, tools_l2_explorer_port: null,
    tools_bridge_ui_port: null, tools_db_port: null, tools_metrics_port: null,
    env_project_id: null, env_updated_at: null,
  });
}

// ============================================================
// HELPERS
// ============================================================

async function waitForHealthy(url, timeoutMs, deploymentId) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await isHealthy(url)) return;
    if (deploymentId) emit(deploymentId, "waiting", { message: `Waiting for ${url} to be ready...` });
    await new Promise((r) => setTimeout(r, 3000));
  }
  throw new Error(`Timeout waiting for ${url} to become healthy`);
}

async function waitForRemoteHealthy(conn, port, timeoutMs, deploymentId) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const { code } = await remote.exec(
        conn,
        `curl -sf -o /dev/null http://127.0.0.1:${port} -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'`,
        { ignoreError: true, timeout: 5000 }
      );
      if (code === 0) return;
    } catch {}
    if (deploymentId) emit(deploymentId, "waiting", { message: `Waiting for remote port ${port}...` });
    await new Promise((r) => setTimeout(r, 3000));
  }
  throw new Error(`Timeout waiting for remote port ${port} to become healthy`);
}

module.exports = {
  provision,
  provisionRemote,
  stopDeployment,
  startDeployment,
  destroyDeployment,
  getEmitter,
  PHASES,
};

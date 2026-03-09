/**
 * Deployment Engine -- orchestrates the full L2 deployment lifecycle.
 *
 * Supports two modes:
 * - Local: builds from source via Docker Compose on the platform host
 * - Remote: uses pre-built images, deploys via SSH to a remote server
 *
 * State machine: configured -> building/pulling -> l1_starting -> deploying_contracts -> l2_starting -> running
 * On error: -> error (with rollback)
 *
 * Features:
 * - Active deployment registry (tracks which provisions are running)
 * - Persistent event/log storage in DB (survives page navigation)
 * - Recovery on server restart (detects stuck deployments)
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
const { updateDeployment, getNextAvailablePorts, getAllDeployments, insertDeployEvent, clearDeployEvents } = require("../db/deployments");
const { getHostById } = require("../db/hosts");

// Active deployments event emitters (keyed by deployment ID)
const deploymentEvents = new Map();

// Active provision registry -- tracks which deployments have a running provision()
const activeProvisions = new Map(); // id -> { startedAt, phase, abortController }

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

const ACTIVE_PHASES = [
  "checking_docker", "building", "pulling", "l1_starting",
  "deploying_contracts", "l2_starting", "starting_prover", "starting_tools",
];

function getEmitter(deploymentId) {
  if (!deploymentEvents.has(deploymentId)) {
    deploymentEvents.set(deploymentId, new EventEmitter());
  }
  return deploymentEvents.get(deploymentId);
}

function emit(deploymentId, event, data) {
  const emitter = deploymentEvents.get(deploymentId);
  const payload = { event, ...data, timestamp: Date.now() };
  if (emitter) {
    emitter.emit("event", payload);
  }
  // Persist to DB (skip if deployment no longer exists)
  try {
    const phase = data?.phase || null;
    const message = data?.message || null;
    const extraData = { ...data };
    delete extraData.event;
    delete extraData.phase;
    delete extraData.message;
    delete extraData.timestamp;
    const hasExtra = Object.keys(extraData).length > 0;
    insertDeployEvent(deploymentId, event, phase, message, hasExtra ? extraData : null);
  } catch (e) {
    // Log once per deployment to avoid spam
    if (!emit._warned) emit._warned = new Set();
    if (!emit._warned.has(deploymentId)) {
      emit._warned.add(deploymentId);
      console.warn(`[deploy-engine] Cannot persist event for ${deploymentId}: ${e.message}`);
    }
  }
}

/** Check if a deployment has an active provision running */
function isProvisionActive(deploymentId) {
  return activeProvisions.has(deploymentId);
}

/** Cancel an active provision (cleanup before delete) */
function cancelProvision(deploymentId) {
  if (activeProvisions.has(deploymentId)) {
    activeProvisions.delete(deploymentId);
    deploymentEvents.delete(deploymentId);
    console.log(`[deploy-engine] Cancelled active provision for ${deploymentId}`);
  }
}

/** Get info about all active provisions */
function getActiveProvisions() {
  const result = [];
  for (const [id, info] of activeProvisions) {
    result.push({ id, startedAt: info.startedAt, phase: info.phase });
  }
  return result;
}

// ============================================================
// LOCAL PROVISIONING (build from source)
// ============================================================

async function provision(deployment) {
  const { id, program_slug: programSlug } = deployment;

  // Register as active
  const provisionInfo = { startedAt: Date.now(), phase: "checking_docker" };
  activeProvisions.set(id, provisionInfo);

  // Clear previous events for a fresh run
  clearDeployEvents(id);

  emit(id, "phase", { phase: "checking_docker", message: "Checking Docker availability..." });
  updateDeployment(id, { phase: "checking_docker", error_message: null });

  if (!docker.isDockerAvailable()) {
    const errMsg = "Docker is not running. Please install and start Docker Desktop first.";
    emit(id, "error", { message: errMsg });
    updateDeployment(id, { phase: "error", error_message: errMsg });
    activeProvisions.delete(id);
    throw new Error(errMsg);
  }

  emit(id, "phase", { phase: "checking_docker", message: "Docker is available" });

  // Parse config
  let deployDir = null;
  let dumpFixtures = false;
  try {
    const config = deployment.config ? JSON.parse(deployment.config) : {};
    deployDir = config.deployDir || null;
    dumpFixtures = !!config.dumpFixtures;
  } catch {}

  const { l1Port, l2Port, proofCoordPort, toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, toolsMetricsPort } = await getNextAvailablePorts();
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

  provisionInfo.phase = "building";
  emit(id, "phase", { phase: "building", message: "Generating Docker Compose configuration..." });

  try {
    const gpu = docker.hasNvidiaGpu();
    const composeContent = generateComposeFile({ programSlug, l1Port, l2Port, proofCoordPort, metricsPort: toolsMetricsPort, projectName, gpu, dumpFixtures });
    const composeFile = writeComposeFile(id, composeContent, deployDir);

    emit(id, "phase", { phase: "building", message: "Building Docker images... (this may take several minutes on first run)" });
    await docker.buildImages(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" }, (chunk) => {
      const lines = chunk.split("\n").filter(Boolean);
      for (const line of lines) {
        emit(id, "log", { message: line });
      }
    });

    provisionInfo.phase = "l1_starting";
    emit(id, "phase", { phase: "l1_starting", message: "Starting L1 node..." });
    updateDeployment(id, { phase: "l1_starting" });
    await docker.startL1(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
    await waitForHealthy(`http://127.0.0.1:${l1Port}`, 60000, id);
    emit(id, "phase", { phase: "l1_starting", message: "L1 node is running" });

    provisionInfo.phase = "deploying_contracts";
    emit(id, "phase", { phase: "deploying_contracts", message: "Deploying L1 contracts (bridge, proposer, verifier)..." });
    updateDeployment(id, { phase: "deploying_contracts" });
    await docker.deployContracts(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });

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
    const timelockAddress = envVars.ETHREX_TIMELOCK_ADDRESS || null;
    const sp1VerifierAddress = envVars.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS || null;
    console.log(`[deployment-engine] extractEnv for ${projectName}: bridge=${bridgeAddress}, proposer=${proposerAddress}, timelock=${timelockAddress}, sp1Verifier=${sp1VerifierAddress}`);
    emit(id, "log", { message: `extractEnv [${projectName}]: bridge=${bridgeAddress}, proposer=${proposerAddress}, timelock=${timelockAddress}, sp1Verifier=${sp1VerifierAddress}` });

    if (!bridgeAddress || !proposerAddress) {
      throw new Error(
        `Contract deployment incomplete: bridge=${bridgeAddress}, proposer=${proposerAddress}. ` +
        "The deployer may have exited before writing contract addresses."
      );
    }

    updateDeployment(id, {
      bridge_address: bridgeAddress,
      proposer_address: proposerAddress,
      timelock_address: timelockAddress,
      sp1_verifier_address: sp1VerifierAddress,
      env_project_id: projectName,
      env_updated_at: Date.now(),
    });

    emit(id, "phase", { phase: "deploying_contracts", message: "Contracts deployed", bridgeAddress, proposerAddress });

    provisionInfo.phase = "l2_starting";
    emit(id, "phase", { phase: "l2_starting", message: "Starting L2 node..." });
    updateDeployment(id, { phase: "l2_starting" });
    await docker.startL2(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
    await waitForHealthy(`http://127.0.0.1:${l2Port}`, 120000, id);
    emit(id, "phase", { phase: "l2_starting", message: "L2 node is running" });

    provisionInfo.phase = "starting_prover";
    emit(id, "phase", { phase: "starting_prover", message: "Starting prover..." });
    updateDeployment(id, { phase: "starting_prover" });
    await docker.startProver(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });

    provisionInfo.phase = "starting_tools";
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
    activeProvisions.delete(id);
    return updateDeployment(id, {});
  } catch (err) {
    emit(id, "error", { message: err.message });
    updateDeployment(id, { phase: "error", error_message: err.message });
    activeProvisions.delete(id);
    // Do NOT auto-destroy Docker containers on error.
    // User can inspect logs/state and manually delete or retry.
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

  const provisionInfo = { startedAt: Date.now(), phase: "pulling" };
  activeProvisions.set(id, provisionInfo);
  clearDeployEvents(id);

  const { l1Port, l2Port, proofCoordPort } = await getNextAvailablePorts();
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

    const composeContent = generateRemoteComposeFile({
      programSlug,
      l1Port,
      l2Port,
      proofCoordPort,
      projectName,
      dataDir: remoteDir,
    });

    writeComposeFile(id, composeContent);

    provisionInfo.phase = "pulling";
    emit(id, "phase", { phase: "pulling", message: "Uploading configuration and pulling images..." });

    await remote.exec(conn, `mkdir -p ${remoteDir}`);
    await remote.uploadFile(conn, composeContent, `${remoteDir}/docker-compose.yaml`);

    const profile = getAppProfile(programSlug);
    if (profile.programsToml) {
      const tomlContent = generateProgramsToml(programSlug);
      await remote.uploadFile(conn, tomlContent, `${remoteDir}/programs.toml`);
    }

    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} pull`, {
      timeout: 300000,
    });

    provisionInfo.phase = "l1_starting";
    emit(id, "phase", { phase: "l1_starting", message: "Starting L1 node on remote server..." });
    updateDeployment(id, { phase: "l1_starting" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d tokamak-app-l1`, {
      timeout: 60000,
    });

    await waitForRemoteHealthy(conn, l1Port, 60000, id);
    emit(id, "phase", { phase: "l1_starting", message: "L1 node is running" });

    provisionInfo.phase = "deploying_contracts";
    emit(id, "phase", { phase: "deploying_contracts", message: "Deploying contracts on remote..." });
    updateDeployment(id, { phase: "deploying_contracts" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up tokamak-app-deployer`, {
      timeout: 600000,
    });

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

    provisionInfo.phase = "l2_starting";
    emit(id, "phase", { phase: "l2_starting", message: "Starting L2 node on remote..." });
    updateDeployment(id, { phase: "l2_starting" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d tokamak-app-l2`, {
      timeout: 60000,
    });
    await waitForRemoteHealthy(conn, l2Port, 120000, id);
    emit(id, "phase", { phase: "l2_starting", message: "L2 node is running" });

    provisionInfo.phase = "starting_prover";
    emit(id, "phase", { phase: "starting_prover", message: "Starting prover on remote..." });
    updateDeployment(id, { phase: "starting_prover" });
    await remote.exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d tokamak-app-prover`, {
      timeout: 60000,
    });

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
    activeProvisions.delete(id);
    conn.end();
    return updateDeployment(id, {});
  } catch (err) {
    emit(id, "error", { message: err.message });
    updateDeployment(id, { phase: "error", error_message: err.message });
    activeProvisions.delete(id);
    // Do NOT auto-destroy remote containers on error.
    // User can inspect logs/state and manually delete or retry.
    if (conn) conn.end();
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
  // Stop tools (Explorer, Bridge UI) first, then the deployment containers
  try { await docker.stopTools(); } catch { /* tools may not be running */ }
  await docker.stop(deployment.docker_project, composeFile);
  return updateDeployment(deployment.id, { phase: "stopped", status: "configured" });
}

async function startDeployment(deployment) {
  if (deployment.host_id) {
    return await startDeploymentRemote(deployment);
  }
  const composeFile = require("path").join(getDeploymentDir(deployment.id), "docker-compose.yaml");
  // Start core services (L1, L2, Prover)
  await docker.start(deployment.docker_project, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
  // Also start tools (Explorer, Bridge UI, Dashboard) if they were provisioned
  try {
    const envVars = await docker.extractEnv(deployment.docker_project, composeFile);
    await docker.startTools(envVars, {
      toolsL1ExplorerPort: deployment.tools_l1_explorer_port,
      toolsL2ExplorerPort: deployment.tools_l2_explorer_port,
      toolsBridgeUIPort: deployment.tools_bridge_ui_port,
      toolsDbPort: deployment.tools_db_port,
      l1Port: deployment.l1_port,
      l2Port: deployment.l2_port,
      toolsMetricsPort: deployment.tools_metrics_port,
    });
  } catch (e) {
    console.log(`[start] Tools start skipped: ${e.message}`);
  }
  return updateDeployment(deployment.id, { phase: "running", status: "active" });
}

async function destroyDeployment(deployment) {
  if (deployment.host_id) {
    return await destroyDeploymentRemote(deployment);
  }
  const composeFile = require("path").join(getDeploymentDir(deployment.id), "docker-compose.yaml");
  // Stop tools (Explorer, Bridge UI) first, then destroy the deployment
  try { await docker.stopTools(); } catch { /* tools may not be running */ }
  await docker.destroy(deployment.docker_project, composeFile);
  const fs = require("fs");
  const deployDir = getDeploymentDir(deployment.id);
  if (fs.existsSync(deployDir)) fs.rmSync(deployDir, { recursive: true, force: true });
  return updateDeployment(deployment.id, {
    phase: "configured", status: "configured",
    docker_project: null, l1_port: null, l2_port: null, proof_coord_port: null,
    bridge_address: null, proposer_address: null, timelock_address: null, sp1_verifier_address: null,
    error_message: null, host_id: null,
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
    bridge_address: null, proposer_address: null, timelock_address: null, sp1_verifier_address: null,
    error_message: null, host_id: null,
    tools_l1_explorer_port: null, tools_l2_explorer_port: null,
    tools_bridge_ui_port: null, tools_db_port: null, tools_metrics_port: null,
    env_project_id: null, env_updated_at: null,
  });
}

// ============================================================
// SERVER STARTUP RECOVERY
// ============================================================

/**
 * Called on server start. Detects deployments stuck in active phases
 * (building, l1_starting, etc.) with no running provision.
 * Marks them as error since the build process was lost.
 */
async function recoverStuckDeployments() {
  try {
    const deployments = getAllDeployments();
    for (const dep of deployments) {
      // Mark stuck active-phase deployments as error
      if (ACTIVE_PHASES.includes(dep.phase) && !activeProvisions.has(dep.id)) {
        console.log(`[recovery] Deployment ${dep.id} (${dep.name}) stuck in phase "${dep.phase}" -- marking as error`);
        const errMsg = `Server restarted while deployment was in "${dep.phase}" phase. The build process was lost. Please retry.`;
        updateDeployment(dep.id, { phase: "error", error_message: errMsg });
        insertDeployEvent(dep.id, "error", dep.phase, errMsg, null);
        continue;
      }
      // Backfill missing contract addresses from Docker env volume
      if (dep.bridge_address && (!dep.timelock_address || !dep.sp1_verifier_address) && dep.docker_project) {
        try {
          const composeFile = require("path").join(getDeploymentDir(dep.id), "docker-compose.yaml");
          const envVars = await docker.extractEnv(dep.docker_project, composeFile);
          const updates = {};
          if (!dep.timelock_address && envVars.ETHREX_TIMELOCK_ADDRESS) updates.timelock_address = envVars.ETHREX_TIMELOCK_ADDRESS;
          if (!dep.sp1_verifier_address && envVars.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS) updates.sp1_verifier_address = envVars.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS;
          if (Object.keys(updates).length > 0) {
            updateDeployment(dep.id, updates);
            console.log(`[recovery] Backfilled contract addresses for ${dep.id}: ${JSON.stringify(updates)}`);
          }
        } catch (e) {
          console.log(`[recovery] Could not backfill contracts for ${dep.id}: ${e.message}`);
        }
      }
      // Reconcile: phase says "running" but Docker containers are actually stopped
      if (dep.phase === "running" && dep.docker_project) {
        try {
          const composeFile = require("path").join(getDeploymentDir(dep.id), "docker-compose.yaml");
          const containers = await docker.getStatus(dep.docker_project, composeFile);
          const anyRunning = containers.some(c => (c.State || "").toLowerCase() === "running");
          if (!anyRunning) {
            console.log(`[recovery] Deployment ${dep.id} (${dep.name}) phase="running" but no containers running -- marking as stopped`);
            updateDeployment(dep.id, { phase: "stopped", status: "configured" });
          }
        } catch (e) {
          console.log(`[recovery] Could not check containers for ${dep.id}: ${e.message}`);
        }
      }
    }
  } catch (e) {
    console.error("[recovery] Failed to recover stuck deployments:", e.message);
  }
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
  isProvisionActive,
  cancelProvision,
  getActiveProvisions,
  recoverStuckDeployments,
  PHASES,
};

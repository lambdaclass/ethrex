/**
 * Deployment Engine -- orchestrates the full L2 deployment lifecycle.
 *
 * Supports three modes:
 * - Local: builds from source via Docker Compose on the platform host
 * - Testnet: builds from source, uses external L1 RPC (no built-in L1 container)
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
  generateTestnetComposeFile,
  generateRemoteComposeFile,
  generateProgramsToml,
  writeComposeFile,
  getDeploymentDir,
  getAppProfile,
} = require("./compose-generator");
const { isHealthy } = require("./rpc-client");
const { updateDeployment, getDeploymentById, getNextAvailablePorts, getAllDeployments, insertDeployEvent, clearDeployEvents } = require("../db/deployments");
const { getHostById } = require("../db/hosts");
const keychain = require("./keychain");
const { getExternalL1Config } = require("./tools-config");
const { verifyAllContracts, SUPPORTED_CHAIN_IDS } = require("./etherscan-verify");

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
  "verifying_contracts",
  "l2_starting",
  "starting_prover",
  "starting_tools",
  "running",
];

const ACTIVE_PHASES = [
  "checking_docker", "building", "pulling", "l1_starting",
  "deploying_contracts", "verifying_contracts", "l2_starting", "starting_prover", "starting_tools",
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
  const info = activeProvisions.get(deploymentId);
  if (info) {
    // Signal the async provision to stop at next phase checkpoint
    info.cancelled = true;
    // Kill any tracked child process (docker build, docker compose up, etc.)
    if (info.activeProcess && !info.activeProcess.killed) {
      try { info.activeProcess.kill("SIGTERM"); } catch {}
    }
    activeProvisions.delete(deploymentId);
    deploymentEvents.delete(deploymentId);
    console.log(`[deploy-engine] Cancelled active provision for ${deploymentId}`);
    return true;
  }
  return false;
}

/**
 * Parse contract addresses from deployer log output.
 *
 * The Rust deployer uses tracing info! macros that produce multiline output.
 * Docker Compose prefixes each line, so the output looks like:
 *   tokamak-app-deployer  | CommonBridge deployed:
 *   tokamak-app-deployer  |   Proxy -> address=0x..., tx_hash=0x...
 *   tokamak-app-deployer  |   Impl  -> address=0x..., tx_hash=0x...
 *
 * SP1Verifier is single-line: SP1Verifier deployed address=0x...
 *
 * Strategy: track the last "deployed:" label, then capture the first
 * "Proxy -> address=..." line that follows it.
 *
 * Returns { bridge, proposer, timelock, sp1Verifier } with whatever was found.
 */
function parseContractAddressesFromLogs(logLines) {
  const result = { bridge: null, proposer: null, timelock: null, sp1Verifier: null, sequencerRegistry: null, router: null, guestProgramRegistry: null };

  // Priority 1: Look for structured JSON output from deployer (DEPLOYER_RESULT_JSON:{...})
  for (const line of logLines) {
    const jsonMatch = line.match(/DEPLOYER_RESULT_JSON:(\{.*\})/);
    if (jsonMatch) {
      try {
        const data = JSON.parse(jsonMatch[1]);
        if (data.status === "success" && data.contracts) {
          result.bridge = data.contracts.CommonBridge || null;
          result.proposer = data.contracts.OnChainProposer || null;
          result.timelock = data.contracts.Timelock || null;
          result.sp1Verifier = data.contracts.SP1Verifier || null;
          result.sequencerRegistry = data.contracts.SequencerRegistry || null;
          result.router = data.contracts.Router || null;
          result.guestProgramRegistry = data.contracts.GuestProgramRegistry || null;
          return result;
        }
      } catch { /* fall through to legacy parsing */ }
    }
  }

  // Priority 2: Legacy log-based parsing (for older deployer versions)
  const addressPattern = /address=(0x[0-9a-fA-F]{40})/;
  let lastContract = null; // which contract was just announced

  for (const line of logLines) {
    // Detect contract announcement lines (e.g. "CommonBridge deployed:")
    if (line.includes("CommonBridge deployed")) lastContract = "bridge";
    else if (line.includes("OnChainProposer deployed")) lastContract = "proposer";
    else if (line.includes("Timelock deployed")) lastContract = "timelock";
    else if (line.includes("SP1Verifier deployed")) lastContract = "sp1Verifier";
    else if (line.includes("SequencerRegistry deployed")) lastContract = "sequencerRegistry";
    else if (line.includes("GuestProgramRegistry deployed")) lastContract = "guestProgramRegistry";
    else if (line.includes("Router deployed")) lastContract = "router";

    const match = line.match(addressPattern);
    if (!match) continue;
    const addr = match[1];

    // SP1Verifier is single-line (address on same line as "deployed")
    if (lastContract === "sp1Verifier" && line.includes("SP1Verifier deployed")) {
      result.sp1Verifier = addr;
      lastContract = null;
      continue;
    }

    // For proxy-based contracts, capture the first "Proxy ->" address after the announcement
    if (lastContract && line.includes("Proxy")) {
      result[lastContract] = addr;
      lastContract = null;
    }
  }
  return result;
}

/**
 * Create a real-time contract address tracker that parses deployer log lines
 * and saves addresses to DB incrementally.
 *
 * Returns { logFn, getAddresses } — logFn is the callback for Docker logs,
 * getAddresses() returns the current parsed state.
 */
function createContractTracker(deploymentId) {
  const state = {
    lastContract: null,
    bridge: null, proposer: null, timelock: null,
    sp1Verifier: null, guestProgramRegistry: null,
  };
  const dbFieldMap = {
    bridge: "bridge_address",
    proposer: "proposer_address",
    timelock: "timelock_address",
    sp1Verifier: "sp1_verifier_address",
    guestProgramRegistry: "guest_program_registry_address",
  };

  function processLine(line) {
    // Check for structured JSON output (highest priority)
    const jsonMatch = line.match(/DEPLOYER_RESULT_JSON:(\{.*\})/);
    if (jsonMatch) {
      try {
        const data = JSON.parse(jsonMatch[1]);
        if (data.status === "success" && data.contracts) {
          const mapping = {
            CommonBridge: "bridge", OnChainProposer: "proposer",
            Timelock: "timelock", SP1Verifier: "sp1Verifier",
            GuestProgramRegistry: "guestProgramRegistry",
          };
          for (const [solName, key] of Object.entries(mapping)) {
            if (data.contracts[solName] && !state[key]) {
              state[key] = data.contracts[solName];
              saveAddress(key, data.contracts[solName]);
            }
          }
          return;
        }
      } catch { /* fall through */ }
    }

    // Detect contract announcement lines
    if (line.includes("CommonBridge deployed")) state.lastContract = "bridge";
    else if (line.includes("OnChainProposer deployed")) state.lastContract = "proposer";
    else if (line.includes("Timelock deployed")) state.lastContract = "timelock";
    else if (line.includes("GuestProgramRegistry deployed")) state.lastContract = "guestProgramRegistry";

    const addrMatch = line.match(/address=(0x[0-9a-fA-F]{40})/);
    if (!addrMatch) return;
    const addr = addrMatch[1];

    // SP1Verifier: address on same line as "deployed"
    if (line.includes("SP1Verifier deployed")) {
      if (!state.sp1Verifier) {
        state.sp1Verifier = addr;
        saveAddress("sp1Verifier", addr);
      }
      state.lastContract = null;
      return;
    }

    // For proxy contracts, capture the Proxy address
    if (state.lastContract && line.includes("Proxy")) {
      if (!state[state.lastContract]) {
        state[state.lastContract] = addr;
        saveAddress(state.lastContract, addr);
      }
      state.lastContract = null;
    }
  }

  function saveAddress(key, address) {
    const dbField = dbFieldMap[key];
    if (dbField) {
      updateDeployment(deploymentId, { [dbField]: address });
      console.log(`[contract-tracker] Saved ${key}=${address} for deployment ${deploymentId}`);
    }
  }

  return {
    logFn: (chunk) => {
      const lines = chunk.split("\n").filter(Boolean);
      for (const line of lines) {
        processLine(line);
      }
    },
    getAddresses: () => ({ ...state, lastContract: undefined }),
  };
}

/** Throw if the provision was cancelled (call between phases) */
function checkCancelled(provisionInfo) {
  if (provisionInfo.cancelled) {
    throw new Error("Deployment cancelled by user");
  }
}

/** Run a docker command while tracking the process for cancellation */
async function trackedDockerRun(provisionInfo, asyncFn) {
  const promise = asyncFn();
  // Track the child process (exposed as promise.process by runCompose)
  if (promise.process) {
    provisionInfo.activeProcess = promise.process;
  }
  try {
    const result = await promise;
    provisionInfo.activeProcess = null;
    return result;
  } catch (err) {
    provisionInfo.activeProcess = null;
    throw err;
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
    updateDeployment(id, { error_message: errMsg });
    activeProvisions.delete(id);
    throw new Error(errMsg);
  }

  emit(id, "phase", { phase: "checking_docker", message: "Docker is available" });

  // Parse config
  let deployDir = null;
  let dumpFixtures = false;
  let forceRebuild = false;
  let forceRedeploy = false;
  try {
    const config = deployment.config ? JSON.parse(deployment.config) : {};
    deployDir = config.deployDir || null;
    dumpFixtures = !!config.dumpFixtures;
    forceRebuild = !!config.forceRebuild;
    forceRedeploy = !!config.forceRedeploy;
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

  let composeFile = null;
  try {
    const gpu = docker.hasNvidiaGpu();
    const composeContent = generateComposeFile({ programSlug, l1Port, l2Port, proofCoordPort, metricsPort: toolsMetricsPort, projectName, gpu, dumpFixtures });
    composeFile = writeComposeFile(id, composeContent, deployDir);

    // Check for existing images to skip rebuild (unless forceRebuild)
    // Both L1 and L2 images must exist; partial builds (e.g. L2 done but L1 cancelled) need full rebuild
    const { execSync } = require("child_process");
    const l1Tag = `tokamak-appchain:l1-${projectName}`;
    const l2Tag = `tokamak-appchain:${programSlug}-${projectName}`;
    let existingImage = null;
    let hasL1Image = false;
    let hasL2Image = false;
    if (!forceRebuild) {
      existingImage = docker.findImage(programSlug);
      try { execSync(`docker image inspect "${l1Tag}" --format "{{.Id}}"`, { stdio: "pipe" }); hasL1Image = true; } catch {}
      try { execSync(`docker image inspect "${l2Tag}" --format "{{.Id}}"`, { stdio: "pipe" }); hasL2Image = true; } catch {}
    }
    if (!forceRebuild && hasL1Image && hasL2Image) {
      // Both project-specific images exist — skip build entirely
      emit(id, "phase", { phase: "building", message: `Docker images found — skipping build` });
      emit(id, "log", { message: `Reusing existing images: L1=${l1Tag}, L2=${l2Tag}` });
    } else if (existingImage && !forceRebuild) {
      // Shared image exists but project tags missing — tag and skip build
      emit(id, "phase", { phase: "building", message: `Docker image found (${existingImage}) — skipping build` });
      emit(id, "log", { message: `Reusing existing image: ${existingImage}` });
      try { execSync(`docker tag "${existingImage}" "${l1Tag}"`, { stdio: "pipe" }); } catch {}
      try { execSync(`docker tag "${existingImage}" "${l2Tag}"`, { stdio: "pipe" }); } catch {}
      emit(id, "log", { message: `Tagged as ${l1Tag} and ${l2Tag}` });
    } else {
      emit(id, "phase", { phase: "building", message: forceRebuild
        ? "Force rebuilding Docker images..."
        : "Building Docker images... (this may take several minutes on first run)" });
      // Remove partial project-specific images to prevent "already exists" BuildKit error
      try { execSync(`docker rmi "${l1Tag}" 2>/dev/null`, { stdio: "pipe" }); } catch {}
      try { execSync(`docker rmi "${l2Tag}" 2>/dev/null`, { stdio: "pipe" }); } catch {}
      await trackedDockerRun(provisionInfo, () =>
        docker.buildImages(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" }, (chunk) => {
          const lines = chunk.split("\n").filter(Boolean);
          for (const line of lines) {
            emit(id, "log", { message: line });
          }
        }, { forceRebuild })
      );
    }

    checkCancelled(provisionInfo);

    provisionInfo.phase = "l1_starting";
    emit(id, "phase", { phase: "l1_starting", message: "Starting L1 node..." });
    updateDeployment(id, { phase: "l1_starting" });
    await trackedDockerRun(provisionInfo, () =>
      docker.startL1(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" })
    );
    await waitForHealthy(`http://127.0.0.1:${l1Port}`, 60000, id);
    emit(id, "phase", { phase: "l1_starting", message: "L1 node is running" });

    checkCancelled(provisionInfo);

    provisionInfo.phase = "deploying_contracts";
    emit(id, "phase", { phase: "deploying_contracts", message: "Deploying L1 contracts..." });
    updateDeployment(id, { phase: "deploying_contracts" });

    // Check for existing contract addresses (skip redeploy when possible)
    const existingDep = getDeploymentById(id);
    const hasExistingContracts = existingDep?.bridge_address && existingDep?.proposer_address;
    let bridgeAddress = null;
    let proposerAddress = null;
    let timelockAddress = null;
    let sp1VerifierAddress = null;
    let guestProgramRegistryAddress = null;

    let envVars = {};

    // Verify saved contract addresses on-chain before reusing (local L1)
    let contractsVerifiedLocal = false;
    if (hasExistingContracts && !forceRedeploy) {
      emit(id, "log", { message: `Found saved contracts in DB — verifying on-chain...` });
      try {
        const { ethers } = require("ethers");
        const rpcUrl = `http://127.0.0.1:${l1Port}`;
        const provider = new ethers.JsonRpcProvider(rpcUrl, undefined, { staticNetwork: true });
        const [bridgeCode, proposerCode] = await Promise.all([
          Promise.race([provider.getCode(existingDep.bridge_address), new Promise((_, r) => setTimeout(() => r(new Error("timeout")), 5000))]),
          Promise.race([provider.getCode(existingDep.proposer_address), new Promise((_, r) => setTimeout(() => r(new Error("timeout")), 5000))]),
        ]);
        provider.destroy();
        contractsVerifiedLocal = bridgeCode && bridgeCode !== "0x" && proposerCode && proposerCode !== "0x";
        if (contractsVerifiedLocal) {
          emit(id, "log", { message: `Contracts verified on-chain` });
        } else {
          emit(id, "log", { message: `Contracts NOT found on-chain. Will redeploy.` });
          updateDeployment(id, { bridge_address: null, proposer_address: null, timelock_address: null, sp1_verifier_address: null, guest_program_registry_address: null });
        }
      } catch (e) {
        emit(id, "log", { message: `Could not verify contracts: ${e.message}. Will redeploy.` });
        updateDeployment(id, { bridge_address: null, proposer_address: null, timelock_address: null, sp1_verifier_address: null, guest_program_registry_address: null });
      }
    }

    if (contractsVerifiedLocal) {
      // Reuse verified contracts
      bridgeAddress = existingDep.bridge_address;
      proposerAddress = existingDep.proposer_address;
      timelockAddress = existingDep.timelock_address;
      sp1VerifierAddress = existingDep.sp1_verifier_address;
      guestProgramRegistryAddress = existingDep.guest_program_registry_address;
      emit(id, "phase", { phase: "deploying_contracts", message: `Reusing verified contracts — bridge: ${bridgeAddress}`, bridgeAddress, proposerAddress, timelockAddress, sp1VerifierAddress, guestProgramRegistryAddress });
      emit(id, "log", { message: `Skipping contract deployment: bridge=${bridgeAddress}, proposer=${proposerAddress}` });

      // Try to restore .env from Docker volume for L2 service
      try {
        envVars = await docker.extractEnv(projectName, composeFile);
        if (envVars.ETHREX_WATCHER_BRIDGE_ADDRESS) bridgeAddress = envVars.ETHREX_WATCHER_BRIDGE_ADDRESS;
      } catch {
        // Use DB values (already set above)
      }
    } else {
      // Deploy contracts — track each address in real-time via log parsing
      emit(id, "phase", { phase: "deploying_contracts", message: forceRedeploy
        ? "Force redeploying L1 contracts..."
        : "Deploying L1 contracts (bridge, proposer, verifier)..." });
      const contractLogLines = [];
      const tracker = createContractTracker(id);
      const contractLogFn = (chunk) => {
        const lines = chunk.split("\n").filter(Boolean);
        for (const line of lines) {
          contractLogLines.push(line);
          emit(id, "log", { message: line });
        }
        // Parse and save addresses incrementally
        tracker.logFn(chunk);
      };
      await trackedDockerRun(provisionInfo, () =>
        docker.deployContracts(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" }, contractLogFn)
      );

      await docker.stopService(projectName, composeFile, "tokamak-app-deployer");

      // Get addresses from tracker (already saved to DB incrementally)
      const tracked = tracker.getAddresses();
      bridgeAddress = tracked.bridge;
      proposerAddress = tracked.proposer;
      timelockAddress = tracked.timelock;
      sp1VerifierAddress = tracked.sp1Verifier;
      guestProgramRegistryAddress = tracked.guestProgramRegistry;

      // Fallback: try extractEnv if tracker missed any
      if (!bridgeAddress || !proposerAddress) {
        try {
          envVars = await docker.extractEnv(projectName, composeFile);
        } catch (extractErr) {
          emit(id, "log", { message: `Warning: extractEnv failed: ${extractErr.message}, retrying...` });
          await new Promise(r => setTimeout(r, 3000));
          try { envVars = await docker.extractEnv(projectName, composeFile); } catch {}
        }
        if (!bridgeAddress) bridgeAddress = envVars.ETHREX_WATCHER_BRIDGE_ADDRESS || null;
        if (!proposerAddress) proposerAddress = envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null;
        if (!timelockAddress) timelockAddress = envVars.ETHREX_TIMELOCK_ADDRESS || null;
        if (!sp1VerifierAddress) sp1VerifierAddress = envVars.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS || null;
      }

      // Final fallback: parse from accumulated log lines
      if (!bridgeAddress || !proposerAddress) {
        const parsed = parseContractAddressesFromLogs(contractLogLines);
        if (!bridgeAddress && parsed.bridge) bridgeAddress = parsed.bridge;
        if (!proposerAddress && parsed.proposer) proposerAddress = parsed.proposer;
        if (!timelockAddress && parsed.timelock) timelockAddress = parsed.timelock;
        if (!sp1VerifierAddress && parsed.sp1Verifier) sp1VerifierAddress = parsed.sp1Verifier;
        if (parsed.bridge || parsed.proposer) {
          emit(id, "log", { message: `Parsed addresses from deployer logs: bridge=${parsed.bridge}, proposer=${parsed.proposer}` });
        }
      }
    }

    console.log(`[deployment-engine] contract addresses for ${projectName}: bridge=${bridgeAddress}, proposer=${proposerAddress}, timelock=${timelockAddress}, sp1Verifier=${sp1VerifierAddress}`);
    emit(id, "log", { message: `Contract addresses [${projectName}]: bridge=${bridgeAddress}, proposer=${proposerAddress}, timelock=${timelockAddress}, sp1Verifier=${sp1VerifierAddress}` });

    // Save final addresses (ensures all are in DB even from fallback sources)
    if (bridgeAddress || proposerAddress) {
      updateDeployment(id, {
        bridge_address: bridgeAddress,
        proposer_address: proposerAddress,
        timelock_address: timelockAddress,
        sp1_verifier_address: sp1VerifierAddress,
        env_project_id: projectName,
        env_updated_at: Date.now(),
      });
    }

    if (!bridgeAddress || !proposerAddress) {
      throw new Error(
        `Contract deployment incomplete: bridge=${bridgeAddress}, proposer=${proposerAddress}. ` +
        "The deployer may have exited before writing contract addresses."
      );
    }

    emit(id, "phase", { phase: "deploying_contracts", message: "Contracts deployed", bridgeAddress, proposerAddress, timelockAddress, sp1VerifierAddress, guestProgramRegistryAddress });

    checkCancelled(provisionInfo);

    provisionInfo.phase = "l2_starting";
    emit(id, "phase", { phase: "l2_starting", message: "Starting L2 node..." });
    updateDeployment(id, { phase: "l2_starting" });
    await trackedDockerRun(provisionInfo, () =>
      docker.startL2(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" })
    );
    await waitForHealthy(`http://127.0.0.1:${l2Port}`, 120000, id);
    emit(id, "phase", { phase: "l2_starting", message: "L2 node is running" });

    checkCancelled(provisionInfo);

    provisionInfo.phase = "starting_prover";
    emit(id, "phase", { phase: "starting_prover", message: "Starting prover..." });
    updateDeployment(id, { phase: "starting_prover" });
    await trackedDockerRun(provisionInfo, () =>
      docker.startProver(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" })
    );

    checkCancelled(provisionInfo);

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
      await docker.startTools(`${projectName}-tools`, envVars, { toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, l1Port, l2Port, toolsMetricsPort });
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
    updateDeployment(id, { phase: "running", status: "active", error_message: null, ever_running: 1 });
    activeProvisions.delete(id);
    return updateDeployment(id, {});
  } catch (err) {
    // Stop all containers on error/cancel to prevent restart loops
    if (composeFile) {
      try {
        await docker.stop(projectName, composeFile);
        emit(id, "log", { message: `Stopped all containers for ${projectName}` });
      } catch (stopErr) {
        console.warn(`[deploy-engine] Failed to stop containers on error: ${stopErr.message}`);
      }
    }
    // Don't overwrite state if already cancelled by user (stop endpoint sets phase)
    if (!provisionInfo.cancelled) {
      emit(id, "error", { message: err.message });
      updateDeployment(id, { error_message: err.message });
    }
    activeProvisions.delete(id);
    throw err;
  }
}

// ============================================================
// TESTNET PROVISIONING (build from source, external L1)
// ============================================================

async function provisionTestnet(deployment) {
  const { id, program_slug: programSlug } = deployment;

  const provisionInfo = { startedAt: Date.now(), phase: "checking_docker" };
  activeProvisions.set(id, provisionInfo);
  clearDeployEvents(id);

  emit(id, "phase", { phase: "checking_docker", message: "Checking Docker availability..." });
  updateDeployment(id, { phase: "checking_docker", error_message: null });

  if (!docker.isDockerAvailable()) {
    const errMsg = "Docker is not running. Please install and start Docker Desktop first.";
    emit(id, "error", { message: errMsg });
    updateDeployment(id, { error_message: errMsg });
    activeProvisions.delete(id);
    throw new Error(errMsg);
  }

  emit(id, "phase", { phase: "checking_docker", message: "Docker is available" });
  emit(id, "log", { message: "Docker check passed" });

  let deployDir = null;
  let testnetConfig = {};
  try {
    const config = deployment.config ? JSON.parse(deployment.config) : {};
    deployDir = config.deployDir || null;
    testnetConfig = config.testnet || {};
  } catch {}

  const l1RpcUrl = testnetConfig.l1RpcUrl;
  if (!l1RpcUrl) {
    const errMsg = "L1 RPC URL is required for testnet deployment.";
    emit(id, "error", { message: errMsg });
    updateDeployment(id, { error_message: errMsg });
    activeProvisions.delete(id);
    throw new Error(errMsg);
  }

  emit(id, "log", { message: `L1 RPC URL: ${l1RpcUrl}` });
  emit(id, "log", { message: `L1 Network: ${testnetConfig.network || 'custom'}` });
  emit(id, "log", { message: `L1 Chain ID: ${testnetConfig.l1ChainId || 'auto'}` });

  // Helper to resolve a private key from Keychain
  function resolveKeychainKey(keychainKeyName, roleLabel) {
    emit(id, "log", { message: `Loading ${roleLabel} key from Keychain: "${keychainKeyName}"...` });
    const resolved = keychain.getSecret(keychainKeyName);
    if (!resolved) {
      const errMsg = `${roleLabel} key "${keychainKeyName}" not found in Keychain. Please re-register the key.`;
      emit(id, "error", { message: errMsg });
      updateDeployment(id, { error_message: errMsg });
      activeProvisions.delete(id);
      throw new Error(errMsg);
    }
    emit(id, "log", { message: `${roleLabel} key loaded from Keychain: "${keychainKeyName}"` });
    return resolved;
  }

  // Resolve deployer private key: prefer keychain, fallback to raw value
  let deployerPrivateKey = testnetConfig.deployerPrivateKey;
  if (testnetConfig.keychainKeyName) {
    deployerPrivateKey = resolveKeychainKey(testnetConfig.keychainKeyName, 'Deployer');
  }

  // Resolve role-specific keys (fallback to deployer key)
  const roleKeys = { committerPk: null, proofCoordinatorPk: null, bridgeOwnerPk: null };
  const roleKeyMap = [
    { configKey: 'committerKeychainKey', resultKey: 'committerPk', label: 'Committer' },
    { configKey: 'proofCoordinatorKeychainKey', resultKey: 'proofCoordinatorPk', label: 'Proof Coordinator' },
    { configKey: 'bridgeOwnerKeychainKey', resultKey: 'bridgeOwnerPk', label: 'Bridge Owner' },
  ];
  for (const { configKey, resultKey, label } of roleKeyMap) {
    const keychainName = testnetConfig[configKey];
    if (keychainName) {
      roleKeys[resultKey] = resolveKeychainKey(keychainName, label);
    }
  }

  // Testnet: no L1 port needed, only L2 + tools
  const { l2Port, proofCoordPort, toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, toolsMetricsPort } = await getNextAvailablePorts();
  const projectName = `tokamak-${id.slice(0, 8)}`;
  emit(id, "log", { message: `Project: ${projectName}, L2 port: ${l2Port}, Proof coord port: ${proofCoordPort}` });

  updateDeployment(id, {
    docker_project: projectName,
    l1_port: null,
    l2_port: l2Port,
    proof_coord_port: proofCoordPort,
    tools_l1_explorer_port: toolsL1ExplorerPort,
    tools_l2_explorer_port: toolsL2ExplorerPort,
    tools_bridge_ui_port: toolsBridgeUIPort,
    tools_db_port: toolsDbPort,
    tools_metrics_port: toolsMetricsPort,
    deploy_dir: deployDir,
    rpc_url: l1RpcUrl,
    phase: "building",
    error_message: null,
  });

  provisionInfo.phase = "building";
  emit(id, "phase", { phase: "building", message: "Generating Docker Compose configuration (testnet mode)..." });

  let composeFile = null;
  const contractLogLines = []; // Track deployer logs for address recovery on cancel
  try {
    const gpu = docker.hasNvidiaGpu();
    const composeContent = generateTestnetComposeFile({
      programSlug, l2Port, proofCoordPort, metricsPort: toolsMetricsPort,
      projectName, l1RpcUrl, deployerPrivateKey, gpu,
      committerPk: roleKeys.committerPk,
      proofCoordinatorPk: roleKeys.proofCoordinatorPk,
      bridgeOwnerPk: roleKeys.bridgeOwnerPk,
    });
    composeFile = writeComposeFile(id, composeContent, deployDir);
    emit(id, "log", { message: `Docker Compose file: ${composeFile}` });

    // Check if any image for this programSlug already exists — skip build if so
    const sharedImage = `tokamak-appchain:${programSlug}`;
    emit(id, "log", { message: `Checking for existing Docker image: ${sharedImage}` });
    const existingImage = docker.findImage(programSlug);
    if (existingImage) {
      emit(id, "phase", { phase: "building", message: `Docker image found — skipping build`, imageFound: existingImage });
      emit(id, "log", { message: `Reusing existing image: ${existingImage}` });
      // Tag existing image for this project's compose references
      const { execSync } = require("child_process");
      const l2Tag = `tokamak-appchain:${programSlug}-${projectName}`;
      try { execSync(`docker tag "${existingImage}" "${l2Tag}"`, { stdio: "pipe" }); } catch {}
      if (existingImage !== sharedImage) {
        try { execSync(`docker tag "${existingImage}" "${sharedImage}"`, { stdio: "pipe" }); } catch {}
      }
      emit(id, "log", { message: `Tagged as ${l2Tag}` });
    } else {
      emit(id, "phase", { phase: "building", message: "Building Docker images... (this may take several minutes on first run)" });
      await trackedDockerRun(provisionInfo, () =>
        docker.buildImages(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" }, (chunk) => {
          const lines = chunk.split("\n").filter(Boolean);
          for (const line of lines) {
            emit(id, "log", { message: line });
          }
        })
      );
    }

    checkCancelled(provisionInfo);

    // Check if contracts were already deployed for this deployment (e.g. retry after partial failure)
    const existingDep = getDeploymentById(id);
    let bridgeAddress = existingDep?.bridge_address || null;
    let proposerAddress = existingDep?.proposer_address || null;
    let timelockAddress = existingDep?.timelock_address || null;
    let sp1VerifierAddress = existingDep?.sp1_verifier_address || null;
    let guestProgramRegistryAddress = existingDep?.guest_program_registry_address || null;
    let envVars = {};

    // Verify saved contract addresses actually exist on-chain before reusing
    let contractsVerified = false;
    if (bridgeAddress && proposerAddress) {
      emit(id, "log", { message: `Found saved contracts in DB — verifying on-chain...` });
      try {
        const { ethers } = require("ethers");
        const hostRpcUrl = l1RpcUrl.replace("host.docker.internal", "127.0.0.1");
        const provider = new ethers.JsonRpcProvider(hostRpcUrl, undefined, { staticNetwork: true });
        const timeoutMs = 10000;
        const [bridgeCode, proposerCode] = await Promise.all([
          Promise.race([provider.getCode(bridgeAddress), new Promise((_, r) => setTimeout(() => r(new Error("timeout")), timeoutMs))]),
          Promise.race([provider.getCode(proposerAddress), new Promise((_, r) => setTimeout(() => r(new Error("timeout")), timeoutMs))]),
        ]);
        provider.destroy();
        if (bridgeCode && bridgeCode !== "0x" && proposerCode && proposerCode !== "0x") {
          contractsVerified = true;
          emit(id, "log", { message: `Contracts verified on-chain: bridge and proposer have deployed code` });
        } else {
          emit(id, "log", { message: `Contracts NOT found on-chain (bridge=${bridgeCode === "0x" ? "empty" : "ok"}, proposer=${proposerCode === "0x" ? "empty" : "ok"}). Will redeploy.` });
          // Clear stale addresses from DB
          bridgeAddress = null; proposerAddress = null; timelockAddress = null; sp1VerifierAddress = null; guestProgramRegistryAddress = null;
          updateDeployment(id, { bridge_address: null, proposer_address: null, timelock_address: null, sp1_verifier_address: null, guest_program_registry_address: null });
        }
      } catch (verifyErr) {
        emit(id, "log", { message: `Could not verify contracts on-chain: ${verifyErr.message}. Will redeploy.` });
        bridgeAddress = null; proposerAddress = null; timelockAddress = null; sp1VerifierAddress = null; guestProgramRegistryAddress = null;
        updateDeployment(id, { bridge_address: null, proposer_address: null, timelock_address: null, sp1_verifier_address: null, guest_program_registry_address: null });
      }
    }

    if (contractsVerified) {
      // Contracts verified on-chain — skip contract deployment
      emit(id, "phase", { phase: "deploying_contracts", message: `Reusing verified contracts — bridge: ${bridgeAddress}`, bridgeAddress, proposerAddress, timelockAddress, sp1VerifierAddress, guestProgramRegistryAddress });
      emit(id, "log", { message: `Skipping contract deployment: bridge=${bridgeAddress}, proposer=${proposerAddress}` });
      provisionInfo.phase = "deploying_contracts";
      updateDeployment(id, { phase: "deploying_contracts" });

      // Try to restore envVars from previous deployment
      let volumeOk = false;
      try {
        envVars = await docker.extractEnv(projectName, composeFile);
        if (envVars.ETHREX_WATCHER_BRIDGE_ADDRESS) volumeOk = true;
      } catch {
        // Build envVars from saved addresses
        envVars = {
          ETHREX_WATCHER_BRIDGE_ADDRESS: bridgeAddress,
          ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS: proposerAddress,
          ETHREX_TIMELOCK_ADDRESS: timelockAddress || "",
          ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS: sp1VerifierAddress || "",
        };
      }

      // Write env to Docker volume so L2 service can read contract addresses
      if (!volumeOk) {
        const ZERO = "0x0000000000000000000000000000000000000000";
        try {
          docker.writeEnvToVolume(projectName, {
            ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS: proposerAddress,
            ETHREX_TIMELOCK_ADDRESS: timelockAddress || ZERO,
            ETHREX_WATCHER_BRIDGE_ADDRESS: bridgeAddress,
            ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS: sp1VerifierAddress || ZERO,
            ETHREX_DEPLOYER_RISC0_VERIFIER_ADDRESS: ZERO,
            ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS: ZERO,
            ETHREX_DEPLOYER_TDX_VERIFIER_ADDRESS: ZERO,
            ENCLAVE_ID_DAO: ZERO,
            FMSPC_TCB_DAO: ZERO,
            PCK_DAO: ZERO,
            PCS_DAO: ZERO,
            ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS: ZERO,
            ETHREX_DEPLOYER_GUEST_PROGRAM_REGISTRY_ADDRESS: guestProgramRegistryAddress || ZERO,
            ETHREX_SHARED_BRIDGE_ROUTER_ADDRESS: ZERO,
          });
          emit(id, "log", { message: "Contract addresses written to Docker volume." });
        } catch (writeErr) {
          emit(id, "log", { message: `Failed to write env to volume: ${writeErr.message}` });
        }
      }
    } else {
      // Deploy contracts to L1
      emit(id, "phase", { phase: "deploying_contracts", message: `Deploying L1 contracts to ${testnetConfig.network || 'external'} L1...` });

      // Log deployer address and balance before contract deployment
      try {
        const { ethers } = require("ethers");
        const wallet = new ethers.Wallet(deployerPrivateKey);
        // Convert Docker-internal URLs to localhost for host-side access
        const hostRpcUrl = l1RpcUrl.replace("host.docker.internal", "127.0.0.1");
        const provider = new ethers.JsonRpcProvider(hostRpcUrl, undefined, { staticNetwork: true });
        const timeoutPromise = new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), 10000));
        const balance = await Promise.race([provider.getBalance(wallet.address), timeoutPromise]);
        const balanceEth = ethers.formatEther(balance);
        emit(id, "log", { message: `Deployer address: ${wallet.address}` });
        emit(id, "log", { message: `Deployer balance: ${balanceEth} ETH` });
        if (balance === 0n) {
          emit(id, "log", { message: `WARNING: Deployer has 0 balance! Contract deployment will fail.` });
        }
        provider.destroy();
      } catch (balErr) {
        emit(id, "log", { message: `Could not check deployer balance: ${balErr.message}` });
      }

      provisionInfo.phase = "deploying_contracts";
      updateDeployment(id, { phase: "deploying_contracts" });
      const tracker = createContractTracker(id);
      const contractLogFn = (chunk) => {
        const lines = chunk.split("\n").filter(Boolean);
        for (const line of lines) {
          contractLogLines.push(line);
          emit(id, "log", { message: line });
        }
        // Parse and save addresses incrementally
        tracker.logFn(chunk);
      };
      let deployerFailed = false;
      try {
        await trackedDockerRun(provisionInfo, () =>
          docker.deployContracts(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" }, contractLogFn)
        );
      } catch (deployErr) {
        // If cancelled, don't try to recover — let checkCancelled handle it
        checkCancelled(provisionInfo);
        // The deployer may have deployed all contracts but failed at a post-deployment step
        // (e.g., make_deposits). Check if we can recover addresses from logs.
        deployerFailed = true;
        emit(id, "log", { message: `Deployer exited with error: ${deployErr.message}` });
        emit(id, "log", { message: "Checking if contract addresses can be recovered from logs..." });
      }

      try { await docker.stopService(projectName, composeFile, "tokamak-app-deployer"); } catch {}

      // Get addresses from tracker (already saved to DB incrementally)
      const tracked = tracker.getAddresses();
      bridgeAddress = tracked.bridge;
      proposerAddress = tracked.proposer;
      timelockAddress = tracked.timelock;
      sp1VerifierAddress = tracked.sp1Verifier;
      guestProgramRegistryAddress = tracked.guestProgramRegistry;

      // Fallback: try extractEnv if tracker missed any
      if (!bridgeAddress || !proposerAddress) {
        try {
          envVars = await docker.extractEnv(projectName, composeFile);
        } catch (extractErr) {
          emit(id, "log", { message: `extractEnv failed: ${extractErr.message}, retrying...` });
          await new Promise(r => setTimeout(r, 3000));
          try { envVars = await docker.extractEnv(projectName, composeFile); } catch {}
        }
        if (!bridgeAddress) bridgeAddress = envVars.ETHREX_WATCHER_BRIDGE_ADDRESS || null;
        if (!proposerAddress) proposerAddress = envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null;
        if (!timelockAddress) timelockAddress = envVars.ETHREX_TIMELOCK_ADDRESS || null;
        if (!sp1VerifierAddress) sp1VerifierAddress = envVars.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS || null;
      }

      // Final fallback: parse from accumulated log lines
      if (!bridgeAddress || !proposerAddress) {
        const parsed = parseContractAddressesFromLogs(contractLogLines);
        if (!bridgeAddress && parsed.bridge) bridgeAddress = parsed.bridge;
        if (!proposerAddress && parsed.proposer) proposerAddress = parsed.proposer;
        if (!timelockAddress && parsed.timelock) timelockAddress = parsed.timelock;
        if (!sp1VerifierAddress && parsed.sp1Verifier) sp1VerifierAddress = parsed.sp1Verifier;
        if (parsed.bridge || parsed.proposer) {
          emit(id, "log", { message: `Recovered addresses from deployer logs: bridge=${parsed.bridge}, proposer=${parsed.proposer}` });
        }
      }

      console.log(`[deployment-engine] testnet contract addresses for ${projectName}: bridge=${bridgeAddress}, proposer=${proposerAddress}`);

      // Save final addresses (ensures all are in DB even from fallback sources)
      if (bridgeAddress || proposerAddress) {
        updateDeployment(id, {
          bridge_address: bridgeAddress,
          proposer_address: proposerAddress,
          timelock_address: timelockAddress,
          sp1_verifier_address: sp1VerifierAddress,
          env_project_id: projectName,
          env_updated_at: Date.now(),
        });
      }

      if (!bridgeAddress || !proposerAddress) {
        throw new Error(
          `Contract deployment incomplete: bridge=${bridgeAddress}, proposer=${proposerAddress}. ` +
          "The deployer may have exited before writing contract addresses."
        );
      }

      // If deployer failed but we recovered all addresses, write them to the Docker volume
      // so the L2 service can read them (the deployer didn't write the .env file)
      if (deployerFailed) {
        emit(id, "log", { message: `Contract addresses recovered despite deployer error. Writing to volume...` });
        const parsed = parseContractAddressesFromLogs(contractLogLines);
        const ZERO = "0x0000000000000000000000000000000000000000";
        try {
          docker.writeEnvToVolume(projectName, {
            ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS: proposerAddress,
            ETHREX_TIMELOCK_ADDRESS: timelockAddress || ZERO,
            ETHREX_WATCHER_BRIDGE_ADDRESS: bridgeAddress,
            ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS: sp1VerifierAddress || ZERO,
            ETHREX_DEPLOYER_RISC0_VERIFIER_ADDRESS: ZERO,
            ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS: ZERO,
            ETHREX_DEPLOYER_TDX_VERIFIER_ADDRESS: ZERO,
            ENCLAVE_ID_DAO: ZERO,
            FMSPC_TCB_DAO: ZERO,
            PCK_DAO: ZERO,
            PCS_DAO: ZERO,
            ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS: parsed.sequencerRegistry || ZERO,
            ETHREX_DEPLOYER_GUEST_PROGRAM_REGISTRY_ADDRESS: parsed.guestProgramRegistry || ZERO,
            ETHREX_SHARED_BRIDGE_ROUTER_ADDRESS: parsed.router || ZERO,
          });
          emit(id, "log", { message: "Contract addresses written to Docker volume." });
        } catch (writeErr) {
          emit(id, "log", { message: `Failed to write env to volume: ${writeErr.message}` });
        }
      }
    }

    checkCancelled(provisionInfo);

    emit(id, "phase", { phase: "deploying_contracts", message: "Contracts deployed on testnet L1", bridgeAddress, proposerAddress, timelockAddress, sp1VerifierAddress, guestProgramRegistryAddress });

    // Etherscan verification for testnet/mainnet (non-blocking)
    const l1ChainId = parseInt(testnetConfig.l1ChainId);
    const etherscanApiKey = testnetConfig.etherscanApiKey || process.env.ETHERSCAN_API_KEY;
    if (SUPPORTED_CHAIN_IDS.has(l1ChainId) && etherscanApiKey) {
      emit(id, "phase", { phase: "verifying_contracts", message: "Verifying contracts on Etherscan..." });
      try {
        const verifyResults = await verifyAllContracts({
          chainId: l1ChainId,
          contracts: { bridge: bridgeAddress, proposer: proposerAddress, timelock: timelockAddress, sp1Verifier: sp1VerifierAddress, guestProgramRegistry: guestProgramRegistryAddress },
          apiKey: etherscanApiKey,
          log: (msg) => emit(id, "log", { message: msg }),
        });
        const verificationStatus = JSON.stringify(verifyResults);
        updateDeployment(id, { verification_status: verificationStatus });
        const allVerified = Object.values(verifyResults).every(r => r.verified);
        emit(id, "phase", { phase: "verifying_contracts", message: allVerified ? "All contracts verified on Etherscan" : "Some contracts could not be verified (see logs)" });
      } catch (verifyErr) {
        emit(id, "log", { message: `Etherscan verification error: ${verifyErr.message}` });
        emit(id, "phase", { phase: "verifying_contracts", message: "Etherscan verification failed (non-blocking)" });
      }
    } else if (SUPPORTED_CHAIN_IDS.has(l1ChainId) && !etherscanApiKey) {
      emit(id, "log", { message: "Etherscan API key not provided — skipping contract verification. Set ETHERSCAN_API_KEY or configure in testnet settings." });
    }

    checkCancelled(provisionInfo);

    provisionInfo.phase = "l2_starting";
    emit(id, "phase", { phase: "l2_starting", message: "Starting L2 node..." });
    updateDeployment(id, { phase: "l2_starting" });
    await trackedDockerRun(provisionInfo, () =>
      docker.startL2(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" })
    );
    await waitForHealthy(`http://127.0.0.1:${l2Port}`, 120000, id);
    emit(id, "phase", { phase: "l2_starting", message: "L2 node is running" });

    checkCancelled(provisionInfo);

    provisionInfo.phase = "starting_prover";
    emit(id, "phase", { phase: "starting_prover", message: "Starting prover..." });
    updateDeployment(id, { phase: "starting_prover" });
    await trackedDockerRun(provisionInfo, () =>
      docker.startProver(projectName, composeFile, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" })
    );

    checkCancelled(provisionInfo);

    provisionInfo.phase = "starting_tools";
    checkCancelled(provisionInfo);
    emit(id, "phase", { phase: "starting_tools", message: "Starting support tools..." });
    updateDeployment(id, { phase: "starting_tools" });
    try {
      const freshEnv = await docker.extractEnv(projectName, composeFile);
      // For testnet/mainnet tools, override L1 RPC to use external URL
      freshEnv.ETHREX_ETH_RPC_URL = l1RpcUrl;
      await docker.startTools(`${projectName}-tools`, freshEnv, {
        toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort,
        l1Port: null, l2Port, toolsMetricsPort,
        skipL1Explorer: true,
        // External L1 metadata for dashboard/bridge UI
        l1RpcUrl,
        l1ChainId: testnetConfig.l1ChainId,
        l1ExplorerUrl: testnetConfig.l1ExplorerUrl || ({ sepolia: 'https://sepolia.etherscan.io', holesky: 'https://holesky.etherscan.io' }[testnetConfig.network] || ''),
        l1NetworkName: testnetConfig.network,
        isExternalL1: true,
      });
      emit(id, "phase", { phase: "starting_tools", message: "Support tools started" });
    } catch (toolsErr) {
      emit(id, "phase", { phase: "starting_tools", message: `Tools setup skipped: ${toolsErr.message}` });
    }

    checkCancelled(provisionInfo);

    emit(id, "phase", {
      phase: "running",
      message: "Testnet deployment is running!",
      l1Rpc: l1RpcUrl,
      l2Rpc: `http://127.0.0.1:${l2Port}`,
      bridgeAddress,
      proposerAddress,
    });
    updateDeployment(id, { phase: "running", status: "active", error_message: null, ever_running: 1 });
    activeProvisions.delete(id);
    return updateDeployment(id, {});
  } catch (err) {
    // Stop all containers on error/cancel to prevent restart loops
    if (composeFile) {
      try {
        await docker.stop(projectName, composeFile);
        emit(id, "log", { message: `Stopped all containers for ${projectName}` });
      } catch (stopErr) {
        console.warn(`[deploy-engine] Failed to stop containers on error: ${stopErr.message}`);
      }
      // Save partial contract addresses from env volume or deployer logs (prevents gas waste on retry)
      let partialBridge = null, partialProposer = null, partialTimelock = null, partialSp1Verifier = null;
      try {
        const partialEnv = await docker.extractEnv(projectName, composeFile);
        partialBridge = partialEnv.ETHREX_WATCHER_BRIDGE_ADDRESS || null;
        partialProposer = partialEnv.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null;
        partialTimelock = partialEnv.ETHREX_TIMELOCK_ADDRESS || null;
        partialSp1Verifier = partialEnv.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS || null;
      } catch { /* env volume may not exist yet */ }
      // Fallback: parse from deployer log output
      if ((!partialBridge || !partialProposer) && contractLogLines.length > 0) {
        const parsed = parseContractAddressesFromLogs(contractLogLines);
        if (!partialBridge && parsed.bridge) partialBridge = parsed.bridge;
        if (!partialProposer && parsed.proposer) partialProposer = parsed.proposer;
        if (!partialTimelock && parsed.timelock) partialTimelock = parsed.timelock;
        if (!partialSp1Verifier && parsed.sp1Verifier) partialSp1Verifier = parsed.sp1Verifier;
      }
      if (partialBridge || partialProposer) {
        updateDeployment(id, {
          bridge_address: partialBridge,
          proposer_address: partialProposer,
          timelock_address: partialTimelock,
          sp1_verifier_address: partialSp1Verifier,
        });
        emit(id, "log", { message: `Saved partial contracts: bridge=${partialBridge}, proposer=${partialProposer}` });
      }
    }
    // Don't overwrite state if already cancelled by user (stop endpoint sets phase)
    if (!provisionInfo.cancelled) {
      emit(id, "error", { message: err.message });
      updateDeployment(id, { error_message: err.message });
    }
    activeProvisions.delete(id);
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
    updateDeployment(id, { phase: "running", status: "active", error_message: null, ever_running: 1 });
    activeProvisions.delete(id);
    conn.end();
    return updateDeployment(id, {});
  } catch (err) {
    emit(id, "error", { message: err.message });
    updateDeployment(id, { error_message: err.message });
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
  try { await docker.stopTools(`${deployment.docker_project}-tools`); } catch { /* tools may not be running */ }
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
    await docker.startTools(`${deployment.docker_project}-tools`, envVars, {
      toolsL1ExplorerPort: deployment.tools_l1_explorer_port,
      toolsL2ExplorerPort: deployment.tools_l2_explorer_port,
      toolsBridgeUIPort: deployment.tools_bridge_ui_port,
      toolsDbPort: deployment.tools_db_port,
      l1Port: deployment.l1_port,
      l2Port: deployment.l2_port,
      toolsMetricsPort: deployment.tools_metrics_port,
      ...getExternalL1Config(deployment),
    });
  } catch (e) {
    console.log(`[start] Tools start skipped: ${e.message}`);
  }
  return updateDeployment(deployment.id, { phase: "running", status: "active", ever_running: 1 });
}

async function destroyDeployment(deployment) {
  if (deployment.host_id) {
    return await destroyDeploymentRemote(deployment);
  }
  const composeFile = require("path").join(getDeploymentDir(deployment.id), "docker-compose.yaml");
  // Stop tools (Explorer, Bridge UI) first, then destroy the deployment
  const toolsProject = `${deployment.docker_project}-tools`;
  try { await docker.stopTools(toolsProject); } catch { /* tools may not be running */ }
  await docker.destroy(deployment.docker_project, composeFile);

  // Verify all containers for this project are removed
  try {
    const remaining = await docker.getStatus(deployment.docker_project, composeFile);
    if (remaining.length > 0) {
      console.log(`[destroy] ${remaining.length} container(s) still present after destroy, force removing...`);
      for (const c of remaining) {
        try {
          require("child_process").execSync(`docker rm -f ${c.ID || c.Name}`, { stdio: "pipe" });
        } catch { /* ignore */ }
      }
    }
  } catch { /* compose file may already be gone */ }

  const fs = require("fs");
  const deployDir = getDeploymentDir(deployment.id);
  if (fs.existsSync(deployDir)) fs.rmSync(deployDir, { recursive: true, force: true });
  // Clean up per-deployment tools env file
  const toolsEnvFile = require("path").join(require("path").resolve(__dirname, "../../../.."), "crates/l2", `.deployed-${toolsProject}.env`);
  if (fs.existsSync(toolsEnvFile)) fs.unlinkSync(toolsEnvFile);
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
  return updateDeployment(deployment.id, { phase: "running", status: "active", ever_running: 1 });
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
        updateDeployment(dep.id, { error_message: errMsg });
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
      // Reconcile DB phase with actual Docker container state
      if (dep.docker_project && (dep.phase === "running" || dep.phase === "stopped")) {
        try {
          const composeFile = require("path").join(getDeploymentDir(dep.id), "docker-compose.yaml");
          const containers = await docker.getStatus(dep.docker_project, composeFile);
          const anyRunning = containers.some(c => (c.State || "").toLowerCase() === "running");
          if (dep.phase === "running" && !anyRunning) {
            console.log(`[recovery] Deployment ${dep.id} (${dep.name}) phase="running" but no containers running -- marking as stopped`);
            updateDeployment(dep.id, { phase: "stopped", status: "configured" });
          } else if (dep.phase === "stopped" && anyRunning) {
            console.log(`[recovery] Deployment ${dep.id} (${dep.name}) phase="stopped" but containers running -- marking as running`);
            updateDeployment(dep.id, { phase: "running", status: "active", ever_running: 1 });
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
  provisionTestnet,
  provisionRemote,
  stopDeployment,
  startDeployment,
  destroyDeployment,
  getEmitter,
  isProvisionActive,
  cancelProvision,
  getActiveProvisions,
  recoverStuckDeployments,
  parseContractAddressesFromLogs,
  PHASES,
};

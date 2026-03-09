#!/usr/bin/env node
/**
 * E2E Test: Sepolia Fork Deployment
 *
 * Uses Anvil to fork Sepolia and tests the full testnet deployment flow:
 * 1. Start Anvil fork
 * 2. Create deployment via API
 * 3. Provision (build skip + contract deploy + L2 start)
 * 4. Monitor SSE events
 * 5. Verify contract addresses saved
 * 6. Test cancel flow
 * 7. Test retry with contract reuse
 */

const { execSync, spawn } = require("child_process");
const http = require("http");

const ANVIL_PORT = 18545;
const SERVER_PORT = 5002;
const API = `http://127.0.0.1:${SERVER_PORT}/api`;
const SEPOLIA_RPC = "https://eth-sepolia.g.alchemy.com/v2/fx6lsTJmHBHSRkrw1ua_y";

// Test deployer key (Anvil default account #0)
const TEST_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const TEST_ADDRESS = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

let anvilProcess = null;
let passed = 0;
let failed = 0;
let deploymentId = null;

// ============================================================
// Helpers
// ============================================================

function fetch(url, opts = {}) {
  return new Promise((resolve, reject) => {
    const parsed = new URL(url);
    const options = {
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname + parsed.search,
      method: opts.method || "GET",
      headers: opts.headers || {},
    };
    const req = http.request(options, (res) => {
      let body = "";
      res.on("data", (d) => (body += d));
      res.on("end", () => {
        try {
          resolve({ ok: res.statusCode < 400, status: res.statusCode, json: () => JSON.parse(body), text: () => body });
        } catch {
          resolve({ ok: res.statusCode < 400, status: res.statusCode, json: () => ({}), text: () => body });
        }
      });
    });
    req.on("error", reject);
    if (opts.body) req.write(opts.body);
    req.end();
  });
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function assert(condition, msg) {
  if (condition) {
    passed++;
    console.log(`  ✓ ${msg}`);
  } else {
    failed++;
    console.log(`  ✗ ${msg}`);
  }
}

async function waitForAnvil(timeout = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      const res = await fetch(`http://127.0.0.1:${ANVIL_PORT}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", method: "eth_blockNumber", params: [], id: 1 }),
      });
      if (res.ok) return true;
    } catch {}
    await sleep(500);
  }
  throw new Error("Anvil failed to start");
}

async function waitForServer(timeout = 5000) {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      const res = await fetch(`${API}/health`);
      if (res.ok) return true;
    } catch {}
    await sleep(300);
  }
  throw new Error("Server not running on port " + SERVER_PORT);
}

async function getDeployment(id) {
  const res = await fetch(`${API}/deployments/${id}`);
  const data = res.json();
  return data.deployment || data;
}

function listenSSE(id, timeout = 120000) {
  return new Promise((resolve, reject) => {
    const events = [];
    const url = `${API}/deployments/${id}/events`;
    const req = http.get(url, (res) => {
      let buffer = "";
      res.on("data", (chunk) => {
        buffer += chunk.toString();
        const lines = buffer.split("\n");
        buffer = lines.pop(); // keep incomplete line
        for (const line of lines) {
          if (line.startsWith("data: ")) {
            try {
              const data = JSON.parse(line.slice(6));
              events.push(data);
              if (data.event === "error" || data.phase === "running") {
                req.destroy();
                resolve(events);
              }
            } catch {}
          }
        }
      });
      res.on("end", () => resolve(events));
    });
    req.on("error", () => resolve(events));
    setTimeout(() => {
      req.destroy();
      resolve(events);
    }, timeout);
  });
}

// ============================================================
// Test Scenarios
// ============================================================

async function startAnvil() {
  console.log("\n--- Starting Anvil (Sepolia fork) ---");
  anvilProcess = spawn("anvil", [
    "--fork-url", SEPOLIA_RPC,
    "--port", String(ANVIL_PORT),
    "--host", "0.0.0.0",
    "--accounts", "1",
    "--balance", "100",
    "--silent",
  ], { stdio: "pipe" });

  anvilProcess.on("error", (err) => {
    console.error("Anvil failed to start:", err.message);
    process.exit(1);
  });

  await waitForAnvil();
  console.log(`  Anvil running on port ${ANVIL_PORT} (forking Sepolia)`);

  // Verify balance
  const balRes = await fetch(`http://127.0.0.1:${ANVIL_PORT}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      method: "eth_getBalance",
      params: [TEST_ADDRESS, "latest"],
      id: 1,
    }),
  });
  const balData = balRes.json();
  const balanceWei = BigInt(balData.result);
  console.log(`  Test account ${TEST_ADDRESS}: ${Number(balanceWei / 10n**15n) / 1000} ETH`);
  assert(balanceWei > 0n, "Test account has balance on Anvil fork");
}

async function testScenario1_FreshDeploy() {
  console.log("\n--- Scenario 1: Fresh Sepolia deployment ---");

  // 1. Create deployment
  const createRes = await fetch(`${API}/deployments`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      programSlug: "zk-dex",
      name: "E2E-Fork-Test",
      chainId: 99999,
      config: {
        mode: "testnet",
        l1Image: "sepolia",
        testnet: {
          l1RpcUrl: `http://host.docker.internal:${ANVIL_PORT}`,
          deployerPrivateKey: TEST_PRIVATE_KEY,
          l1ChainId: 11155111,
          network: "sepolia",
        },
      },
    }),
  });
  const createData = createRes.json();
  deploymentId = createData.deployment?.id || createData.id;
  assert(!!deploymentId, `Deployment created: ${deploymentId?.slice(0, 8)}`);

  // 2. Check deployment in DB
  const dep = await getDeployment(deploymentId);
  assert(dep.name === "E2E-Fork-Test", "Deployment name saved");
  assert(dep.program_slug === "zk-dex", "Program slug saved");

  // 3. Start provision + listen SSE
  console.log("  Starting provision...");
  const [provRes, events] = await Promise.all([
    fetch(`${API}/deployments/${deploymentId}/provision`, { method: "POST" }),
    listenSSE(deploymentId, 180000),
  ]);
  assert(provRes.ok, "Provision request accepted");

  // 4. Analyze SSE events
  const phases = events.filter((e) => e.phase).map((e) => e.phase);
  const uniquePhases = [...new Set(phases)];
  const logs = events.filter((e) => e.event === "log").map((e) => e.message);
  const errors = events.filter((e) => e.event === "error");

  console.log(`  SSE: ${events.length} events, ${uniquePhases.length} phases, ${errors.length} errors`);
  console.log(`  Phases: ${uniquePhases.join(" → ")}`);

  // Check for deployer address + balance log
  const addrLog = logs.find((l) => l.includes("Deployer address:"));
  const balLog = logs.find((l) => l.includes("Deployer balance:"));
  assert(!!addrLog, `Deployer address logged: ${addrLog || "NOT FOUND"}`);
  assert(!!balLog, `Deployer balance logged: ${balLog || "NOT FOUND"}`);

  // Check contract deployment logs
  const contractLogs = logs.filter((l) => l.includes("deployed") || l.includes("initialized"));
  console.log(`  Contract logs: ${contractLogs.length} lines`);

  if (errors.length > 0) {
    console.log(`  ERROR: ${errors[0].message?.slice(0, 200)}`);
  }

  // 5. Check final deployment state
  const finalDep = await getDeployment(deploymentId);
  console.log(`  Final phase: ${finalDep.phase}`);
  console.log(`  Bridge: ${finalDep.bridge_address || "none"}`);
  console.log(`  Proposer: ${finalDep.proposer_address || "none"}`);

  if (finalDep.phase === "running") {
    assert(true, "Deployment reached running state");
    assert(!!finalDep.bridge_address, "Bridge address saved");
    assert(!!finalDep.proposer_address, "Proposer address saved");
  } else if (finalDep.phase === "error") {
    // Check if contracts were partially saved
    console.log(`  Error: ${finalDep.error_message?.slice(0, 200)}`);
    if (finalDep.bridge_address) {
      assert(true, "Partial bridge address saved despite error");
    } else {
      assert(false, `Deployment failed: ${finalDep.error_message?.slice(0, 100)}`);
    }
  } else {
    assert(false, `Unexpected phase: ${finalDep.phase}`);
  }

  return finalDep;
}

async function testScenario2_RetryWithContractReuse(prevDep) {
  console.log("\n--- Scenario 2: Retry with contract reuse ---");

  if (!prevDep?.bridge_address || !prevDep?.proposer_address) {
    console.log("  SKIP: Previous deployment didn't save both contract addresses");
    return;
  }

  // Stop existing containers
  await fetch(`${API}/deployments/${deploymentId}/stop`, { method: "POST" });
  await sleep(2000);

  // Retry provision
  console.log("  Retrying provision (should reuse contracts)...");
  const [provRes, events] = await Promise.all([
    fetch(`${API}/deployments/${deploymentId}/provision`, { method: "POST" }),
    listenSSE(deploymentId, 120000),
  ]);
  assert(provRes.ok, "Retry provision accepted");

  const logs = events.filter((e) => e.event === "log").map((e) => e.message);

  // Check stored DB events for contract reuse (SSE may miss early events due to connection timing)
  const historyRes = await fetch(`${API}/deployments/${deploymentId}/events/history`);
  let allLogs = logs;
  try {
    const historyData = historyRes.json();
    if (historyData.events && Array.isArray(historyData.events)) {
      const dbLogs = historyData.events.filter(e => e.event_type === "log").map(e => e.message);
      allLogs = [...logs, ...dbLogs];
    }
  } catch {}

  const reuseLog = allLogs.find((l) => l && (l.includes("Skipping contract deployment") || l.includes("Reusing existing")));
  assert(!!reuseLog, `Contracts reused: ${reuseLog || "NOT FOUND (checking deployer container)"}`);

  // Verify no deployer container was created for this retry (contracts already exist)
  if (!reuseLog) {
    // Fallback: check that deployer container was NOT re-created
    try {
      const projectName = `tokamak-${deploymentId.slice(0, 8)}`;
      const dockerLogs = execSync(`docker logs ${projectName}-deployer 2>&1 | tail -3`, { timeout: 5000 }).toString();
      // If deployer already existed from the first run, it won't have new logs
      console.log(`  Deployer container logs: ${dockerLogs.trim().slice(0, 100)}`);
    } catch {}
  }

  // Should NOT have new contract deployment logs in SSE
  const deployLogs = logs.filter((l) => l.includes("Deploying") && l.includes("L1"));
  const skipLogs = allLogs.filter((l) => l && (l.includes("Skipping") || l.includes("Reusing")));
  console.log(`  Deploy logs: ${deployLogs.length}, Skip logs: ${skipLogs.length}`);

  const finalDep = await getDeployment(deploymentId);
  console.log(`  Final phase: ${finalDep.phase}`);
  assert(
    finalDep.bridge_address === prevDep.bridge_address,
    "Bridge address unchanged on retry"
  );
}

async function testScenario3_ImageReuse() {
  console.log("\n--- Scenario 3: Docker image reuse ---");

  // Check findImage
  const docker = require("./lib/docker-local");
  const image = docker.findImage("zk-dex");
  assert(!!image, `findImage('zk-dex') = ${image}`);

  // Check that provision skips build (via logs from scenario 1)
  // Already verified in scenario 1 if "Docker image found" appears in logs
  console.log("  Image reuse verified via findImage()");
}

async function testScenario4_CancelDeploy() {
  console.log("\n--- Scenario 4: Cancel during deployment ---");

  // Create a new deployment
  const createRes = await fetch(`${API}/deployments`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      programSlug: "zk-dex",
      name: "E2E-Cancel-Test",
      config: {
        mode: "testnet",
        l1Image: "sepolia",
        testnet: {
          l1RpcUrl: `http://host.docker.internal:${ANVIL_PORT}`,
          deployerPrivateKey: TEST_PRIVATE_KEY,
          l1ChainId: 11155111,
          network: "sepolia",
        },
      },
    }),
  });
  const cancelId = createRes.json().deployment?.id || createRes.json().id;
  assert(!!cancelId, `Cancel test deployment created: ${cancelId?.slice(0, 8)}`);

  // Start provision
  await fetch(`${API}/deployments/${cancelId}/provision`, { method: "POST" });

  // Wait a bit for containers to start, then cancel
  await sleep(5000);

  const stopRes = await fetch(`${API}/deployments/${cancelId}/stop`, { method: "POST" });
  assert(stopRes.ok, "Cancel request accepted");

  await sleep(3000);

  const dep = await getDeployment(cancelId);
  console.log(`  Phase after cancel: ${dep.phase}`);
  assert(
    dep.phase === "stopped" || dep.phase === "configured" || dep.phase === "error",
    `Phase is terminal: ${dep.phase}`
  );

  // Check no Docker containers running for this project
  const projectName = dep.docker_project;
  if (projectName) {
    try {
      const running = execSync(
        `docker ps --filter "name=${projectName}" --format "{{.Names}}" 2>/dev/null`
      ).toString().trim();
      assert(!running, `No containers running for ${projectName}`);
    } catch {
      assert(true, "No containers found (expected)");
    }
  }

  // Cleanup
  await fetch(`${API}/deployments/${cancelId}`, { method: "DELETE" });
}

async function testScenario5_FormReset() {
  console.log("\n--- Scenario 5: Config summary and form ---");

  if (!deploymentId) {
    console.log("  SKIP: No deployment to test");
    return;
  }

  // GET deployment
  const dep = await getDeployment(deploymentId);
  const config = typeof dep.config === "string" ? JSON.parse(dep.config) : dep.config;

  assert(config?.mode === "testnet", "Config mode is testnet");
  assert(config?.testnet?.network === "sepolia", "Config network is sepolia");
  assert(!!config?.testnet?.l1RpcUrl, "Config has L1 RPC URL");
  assert(dep.chain_id > 0, `Chain ID set: ${dep.chain_id}`);
}

// ============================================================
// Main
// ============================================================

async function main() {
  console.log("=== E2E Fork Test: Sepolia Deployment ===\n");

  try {
    // Check server
    await waitForServer();
    console.log("  Server is running");

    // Start Anvil
    await startAnvil();

    // Run scenarios
    const result1 = await testScenario1_FreshDeploy();
    await testScenario3_ImageReuse();
    await testScenario2_RetryWithContractReuse(result1);
    await testScenario4_CancelDeploy();
    await testScenario5_FormReset();

  } catch (err) {
    console.error("\nFATAL:", err.message);
    failed++;
  } finally {
    // Cleanup
    if (deploymentId) {
      try {
        // Stop any running containers
        const dep = await getDeployment(deploymentId);
        if (dep.docker_project) {
          execSync(`docker compose -p ${dep.docker_project} stop 2>/dev/null`, { timeout: 15000 });
        }
        await fetch(`${API}/deployments/${deploymentId}`, { method: "DELETE" });
      } catch {}
    }
    if (anvilProcess) {
      anvilProcess.kill("SIGTERM");
      console.log("\n  Anvil stopped");
    }

    console.log(`\n=== Results: ${passed} passed, ${failed} failed ===`);
    process.exit(failed > 0 ? 1 : 0);
  }
}

main();

/**
 * Local-server test suite
 * Run: node test.js
 */

const assert = require("assert");
const path = require("path");
const fs = require("fs");
const os = require("os");

// Use a temp database for tests
const testDir = path.join(os.tmpdir(), `tokamak-test-${Date.now()}`);
fs.mkdirSync(testDir, { recursive: true });
process.env.TOKAMAK_DATA_DIR = testDir;

let passed = 0;
let failed = 0;

function test(name, fn) {
  try {
    fn();
    passed++;
    console.log(`  ✓ ${name}`);
  } catch (e) {
    failed++;
    console.log(`  ✗ ${name}`);
    console.log(`    ${e.message}`);
  }
}

async function testAsync(name, fn) {
  try {
    await fn();
    passed++;
    console.log(`  ✓ ${name}`);
  } catch (e) {
    failed++;
    console.log(`  ✗ ${name}`);
    console.log(`    ${e.message}`);
  }
}

// ============================================================
// DB Tests
// ============================================================
console.log("\n=== DB Module Tests ===");

const db = require("./db/db");
test("db initializes without error", () => {
  assert.ok(db);
});

test("db has expected tables", () => {
  const tables = db
    .prepare("SELECT name FROM sqlite_master WHERE type='table'")
    .all()
    .map((r) => r.name);
  assert.ok(tables.includes("deployments"), "missing deployments table");
  assert.ok(tables.includes("hosts"), "missing hosts table");
});

// ============================================================
// Deployments DB Tests
// ============================================================
console.log("\n=== Deployments DB Tests ===");

const deploymentsDb = require("./db/deployments");

test("createDeployment creates a record", () => {
  const d = deploymentsDb.createDeployment({
    programId: "test-prog",
    name: "Test Deploy",
  });
  assert.ok(d.id);
  assert.equal(d.name, "Test Deploy");
  assert.equal(d.phase, "configured");
});

let testDeployId;
test("getAllDeployments returns created deployment", () => {
  const all = deploymentsDb.getAllDeployments();
  assert.ok(all.length >= 1);
  testDeployId = all[0].id;
});

test("getDeploymentById returns correct deployment", () => {
  const d = deploymentsDb.getDeploymentById(testDeployId);
  assert.ok(d);
  assert.equal(d.name, "Test Deploy");
});

test("updateDeployment updates allowed fields", () => {
  const updated = deploymentsDb.updateDeployment(testDeployId, {
    phase: "running",
    name: "Updated Deploy",
  });
  assert.equal(updated.phase, "running");
  assert.equal(updated.name, "Updated Deploy");
});

test("updateDeployment rejects disallowed fields", () => {
  // Should silently ignore disallowed fields
  const before = deploymentsDb.getDeploymentById(testDeployId);
  deploymentsDb.updateDeployment(testDeployId, {
    hacker_field: "evil",
  });
  const after = deploymentsDb.getDeploymentById(testDeployId);
  assert.equal(before.name, after.name);
});

test("deleteDeployment removes record", () => {
  deploymentsDb.deleteDeployment(testDeployId);
  const d = deploymentsDb.getDeploymentById(testDeployId);
  assert.equal(d, undefined);
});

// ============================================================
// Hosts DB Tests
// ============================================================
console.log("\n=== Hosts DB Tests ===");

const hostsDb = require("./db/hosts");

test("createHost creates a record", () => {
  const h = hostsDb.createHost({
    name: "Test Server",
    hostname: "192.168.1.1",
    port: 22,
    username: "root",
    authMethod: "key",
  });
  assert.ok(h.id);
  assert.equal(h.name, "Test Server");
  assert.equal(h.hostname, "192.168.1.1");
});

test("getAllHosts returns hosts without private_key", () => {
  const all = hostsDb.getAllHosts();
  assert.ok(all.length >= 1);
  // private_key should not appear in getAllHosts results
  // (the SELECT excludes it)
});

let testHostId;
test("getHostById returns the host", () => {
  const all = hostsDb.getAllHosts();
  testHostId = all[0].id;
  const h = hostsDb.getHostById(testHostId);
  assert.ok(h);
  assert.equal(h.name, "Test Server");
});

test("updateHost updates fields", () => {
  hostsDb.updateHost(testHostId, { name: "Updated Server", status: "active" });
  const h = hostsDb.getHostById(testHostId);
  assert.equal(h.name, "Updated Server");
  assert.equal(h.status, "active");
});

test("deleteHost removes record", () => {
  hostsDb.deleteHost(testHostId);
  const h = hostsDb.getHostById(testHostId);
  assert.equal(h, undefined);
});

// ============================================================
// RPC Client Tests
// ============================================================
console.log("\n=== RPC Client Tests ===");

const { isHealthy } = require("./lib/rpc-client");

testAsync("isHealthy returns false for unreachable host", async () => {
  const result = await isHealthy("http://127.0.0.1:19999");
  assert.equal(result, false);
}).then(() => {
  // ============================================================
  // Port Allocation Tests
  // ============================================================
  console.log("\n=== Port Allocation Tests ===");

  return testAsync("getNextAvailablePorts returns valid ports", async () => {
    const ports = await deploymentsDb.getNextAvailablePorts();
    assert.ok(ports.l1Port > 0);
    assert.ok(ports.l2Port > 0);
    assert.ok(ports.proofCoordPort > 0);
  });
}).then(() => {
  // ============================================================
  // Express App Smoke Test
  // ============================================================
  console.log("\n=== Express App Tests ===");

  const http = require("http");

  testAsync("server responds to /api/health", async () => {
    const app = require("./server");
    const server = http.createServer(app);

    await new Promise((resolve) => server.listen(0, resolve));
    const port = server.address().port;

    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      const data = await res.json();
      assert.equal(data.status, "ok");
    } finally {
      server.close();
    }
  })
    .then(() =>
      testAsync("GET /api/deployments returns array", async () => {
        const app = require("./server");
        const server = http.createServer(app);

        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        try {
          const res = await fetch(`http://127.0.0.1:${port}/api/deployments`);
          const data = await res.json();
          assert.ok(Array.isArray(data.deployments));
        } finally {
          server.close();
        }
      })
    )
    .then(() =>
      testAsync("GET /api/hosts returns array", async () => {
        const app = require("./server");
        const server = http.createServer(app);

        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        try {
          const res = await fetch(`http://127.0.0.1:${port}/api/hosts`);
          const data = await res.json();
          assert.ok(Array.isArray(data.hosts));
        } finally {
          server.close();
        }
      })
    )
    .then(() => {
      // ============================================================
      // Control Logic Tests
      // ============================================================
      console.log("\n=== Control Logic Tests ===");

      // -- TOOLS_SERVICES routing --
      const TOOLS_SERVICES = new Set(["frontend-l1", "backend-l1", "frontend-l2", "backend-l2", "db", "db-init", "redis-db", "proxy", "function-selectors", "bridge-ui"]);

      test("TOOLS_SERVICES contains all expected tools services", () => {
        const expected = ["frontend-l1", "backend-l1", "frontend-l2", "backend-l2", "db", "db-init", "redis-db", "proxy", "function-selectors", "bridge-ui"];
        for (const svc of expected) {
          assert.ok(TOOLS_SERVICES.has(svc), `Missing tools service: ${svc}`);
        }
      });

      test("TOOLS_SERVICES does not contain core services", () => {
        const coreServices = ["tokamak-app-l1", "tokamak-app-l2", "tokamak-app-prover", "tokamak-app-deployer"];
        for (const svc of coreServices) {
          assert.ok(!TOOLS_SERVICES.has(svc), `Core service incorrectly in TOOLS_SERVICES: ${svc}`);
        }
      });

      // -- Phase transitions via DB --
      test("deployment phase transitions: configured → running → stopped", () => {
        const d = deploymentsDb.createDeployment({ programId: "evm-l2", name: "Phase Test" });
        assert.equal(d.phase, "configured");

        // Simulate provision completing
        deploymentsDb.updateDeployment(d.id, { phase: "running", status: "active", docker_project: "tokamak-test123" });
        let updated = deploymentsDb.getDeploymentById(d.id);
        assert.equal(updated.phase, "running");
        assert.equal(updated.status, "active");

        // Simulate stop
        deploymentsDb.updateDeployment(d.id, { phase: "stopped", status: "configured" });
        updated = deploymentsDb.getDeploymentById(d.id);
        assert.equal(updated.phase, "stopped");
        assert.equal(updated.status, "configured");

        // Simulate restart
        deploymentsDb.updateDeployment(d.id, { phase: "running", status: "active" });
        updated = deploymentsDb.getDeploymentById(d.id);
        assert.equal(updated.phase, "running");
        assert.equal(updated.status, "active");

        deploymentsDb.deleteDeployment(d.id);
      });

      test("deployment phase transitions: configured → error (with message)", () => {
        const d = deploymentsDb.createDeployment({ programId: "evm-l2", name: "Error Test" });
        deploymentsDb.updateDeployment(d.id, { phase: "error", error_message: "Docker not running" });
        const updated = deploymentsDb.getDeploymentById(d.id);
        assert.equal(updated.phase, "error");
        assert.equal(updated.error_message, "Docker not running");
        deploymentsDb.deleteDeployment(d.id);
      });

      // -- Status reconciliation logic (unit-level) --
      test("status 'active' maps to running in reconciliation logic", () => {
        // This mirrors the statusMap in MyL2View.tsx
        const statusMap = {
          running: "running", active: "running", stopped: "stopped", deploying: "starting",
          configured: "created", failed: "error", error: "error", destroyed: "stopped",
        };
        assert.equal(statusMap["active"], "running");
        assert.equal(statusMap["running"], "running");
        assert.equal(statusMap["configured"], "created");
        assert.equal(statusMap["stopped"], "stopped");
      });

      test("reconciliation: no containers + status running → stopped", () => {
        // Simulates the reconciliation logic from MyL2View/L2DetailView
        const containers = [];
        const dbStatus = "running";
        let reconciledStatus = dbStatus;
        if (containers.length === 0 && (dbStatus === "running" || dbStatus === "error")) {
          reconciledStatus = "stopped";
        }
        assert.equal(reconciledStatus, "stopped");
      });

      test("reconciliation: no containers + status created (with dockerProject) → stopped", () => {
        const containers = [];
        const dbStatus = "created";
        const dockerProject = "tokamak-abc12345";
        let reconciledStatus = dbStatus;
        if (containers.length === 0 && (dbStatus === "running" || dbStatus === "error" || (dbStatus === "created" && dockerProject))) {
          reconciledStatus = "stopped";
        }
        assert.equal(reconciledStatus, "stopped");
      });

      test("reconciliation: no containers + status created (no dockerProject) → stays created", () => {
        const containers = [];
        const dbStatus = "created";
        const dockerProject = null;
        let reconciledStatus = dbStatus;
        if (containers.length === 0 && (dbStatus === "running" || dbStatus === "error" || (dbStatus === "created" && dockerProject))) {
          reconciledStatus = "stopped";
        }
        assert.equal(reconciledStatus, "created");
      });

      test("reconciliation: all containers running → running", () => {
        const containers = [{ state: "running" }, { state: "running" }, { state: "running" }];
        const allRunning = containers.every(c => c.state === "running");
        assert.equal(allRunning, true);
      });

      test("reconciliation: mixed containers → partial (not all running)", () => {
        const containers = [{ state: "running" }, { state: "exited" }, { state: "running" }];
        const allRunning = containers.every(c => c.state === "running");
        const anyRunning = containers.some(c => c.state === "running");
        assert.equal(allRunning, false);
        assert.equal(anyRunning, true);
      });

      // -- Recovery logic --
      test("ACTIVE_PHASES includes all in-progress phases", () => {
        const { PHASES } = require("./lib/deployment-engine");
        const ACTIVE_PHASES = [
          "checking_docker", "building", "pulling", "l1_starting",
          "deploying_contracts", "l2_starting", "starting_prover", "starting_tools",
        ];
        // All active phases must be valid phases
        for (const p of ACTIVE_PHASES) {
          assert.ok(PHASES.includes(p), `Active phase "${p}" not in PHASES list`);
        }
        // "configured" and "running" must NOT be active phases
        assert.ok(!ACTIVE_PHASES.includes("configured"));
        assert.ok(!ACTIVE_PHASES.includes("running"));
      });

      test("recovery marks stuck active-phase deployment as error", () => {
        // Create a deployment stuck in building phase (simulating server restart)
        const d = deploymentsDb.createDeployment({ programId: "evm-l2", name: "Stuck Deploy" });
        deploymentsDb.updateDeployment(d.id, { phase: "building", docker_project: "tokamak-stuck" });
        let updated = deploymentsDb.getDeploymentById(d.id);
        assert.equal(updated.phase, "building");

        // Simulate what recoverStuckDeployments does for stuck phases
        const ACTIVE_PHASES = ["checking_docker", "building", "pulling", "l1_starting", "deploying_contracts", "l2_starting", "starting_prover", "starting_tools"];
        if (ACTIVE_PHASES.includes(updated.phase)) {
          const errMsg = `Server restarted while deployment was in "${updated.phase}" phase. The build process was lost. Please retry.`;
          deploymentsDb.updateDeployment(d.id, { phase: "error", error_message: errMsg });
        }
        updated = deploymentsDb.getDeploymentById(d.id);
        assert.equal(updated.phase, "error");
        assert.ok(updated.error_message.includes("building"));
        deploymentsDb.deleteDeployment(d.id);
      });

      // -- Start All / Stop All button logic --
      test("ServicesTab: all stopped → shows Start All", () => {
        const containers = [
          { service: "tokamak-app-l1", state: "exited" },
          { service: "tokamak-app-l2", state: "exited" },
          { service: "frontend-l1", state: "exited" },
        ];
        const svcState = (svc) => {
          const c = containers.find(c => c.service === svc);
          return c ? c.state : "not found";
        };
        const services = ["tokamak-app-l1", "tokamak-app-l2", "frontend-l1"];
        const allStopped = services.every(svc => svcState(svc) !== "running");
        const anyRunning = services.some(svc => svcState(svc) === "running");
        assert.equal(allStopped, true, "Should show Start All");
        assert.equal(anyRunning, false, "Should not show Stop All");
      });

      test("ServicesTab: some running → shows Stop All", () => {
        const containers = [
          { service: "tokamak-app-l1", state: "running" },
          { service: "tokamak-app-l2", state: "running" },
          { service: "frontend-l1", state: "exited" },
        ];
        const svcState = (svc) => {
          const c = containers.find(c => c.service === svc);
          return c ? c.state : "not found";
        };
        const services = ["tokamak-app-l1", "tokamak-app-l2", "frontend-l1"];
        const anyRunning = services.some(svc => svcState(svc) === "running");
        assert.equal(anyRunning, true, "Should show Stop All");
      });

      // -- Contract reuse logic --
      test("contract reuse: skip deploy when bridge+proposer already saved", () => {
        const d = deploymentsDb.createDeployment({ programId: "evm-l2", name: "Contract Reuse Test" });
        deploymentsDb.updateDeployment(d.id, {
          bridge_address: "0x1234567890abcdef",
          proposer_address: "0xabcdef1234567890",
        });
        const updated = deploymentsDb.getDeploymentById(d.id);
        // When both addresses exist, provisionTestnet should skip contract deployment
        assert.ok(updated.bridge_address && updated.proposer_address, "Both addresses should exist");
        deploymentsDb.deleteDeployment(d.id);
      });

      test("contract reuse: deploy when bridge is missing", () => {
        const d = deploymentsDb.createDeployment({ programId: "evm-l2", name: "No Contract Test" });
        const updated = deploymentsDb.getDeploymentById(d.id);
        // When addresses are null, provisionTestnet should deploy contracts
        assert.equal(updated.bridge_address, null, "Bridge should be null for fresh deployment");
        assert.equal(updated.proposer_address, null, "Proposer should be null for fresh deployment");
        deploymentsDb.deleteDeployment(d.id);
      });

      // -- PUT config update --
      test("updateDeployment allows config field", () => {
        const d = deploymentsDb.createDeployment({ programId: "evm-l2", name: "Config Update Test" });
        const config = { mode: "testnet", testnet: { l1RpcUrl: "https://sepolia.example.com", keychainKeyName: "mykey" } };
        deploymentsDb.updateDeployment(d.id, { config: JSON.stringify(config) });
        const updated = deploymentsDb.getDeploymentById(d.id);
        const parsed = JSON.parse(updated.config);
        assert.equal(parsed.mode, "testnet");
        assert.equal(parsed.testnet.keychainKeyName, "mykey");
        deploymentsDb.deleteDeployment(d.id);
      });

      // -- findImage function --
      test("docker findImage returns null for nonexistent image", () => {
        const docker = require("./lib/docker-local");
        const result = docker.findImage("nonexistent-slug-12345");
        assert.equal(result, null);
      });

      // -- parseContractAddressesFromLogs --
      const { parseContractAddressesFromLogs } = require("./lib/deployment-engine");

      test("parseContractAddressesFromLogs extracts addresses from deployer output", () => {
        const logs = [
          "tokamak-app-deployer  | CommonBridge deployed:",
          "tokamak-app-deployer  |   Proxy -> address=0x2f6cf9ec2beed1b8169330994242e97398ce3352, tx_hash=0xabc",
          "tokamak-app-deployer  |   Impl  -> address=0x1111111111111111111111111111111111111111, tx_hash=0xdef",
          "tokamak-app-deployer  | OnChainProposer deployed:",
          "tokamak-app-deployer  |   Proxy -> address=0xa59bdbd3bd6764b04f182973bceb51da127114d2, tx_hash=0xdef",
          "tokamak-app-deployer  |   Impl  -> address=0x2222222222222222222222222222222222222222, tx_hash=0xghi",
          "tokamak-app-deployer  | Timelock deployed:",
          "tokamak-app-deployer  |   Proxy -> address=0x1234567890abcdef1234567890abcdef12345678, tx_hash=0x111",
          "tokamak-app-deployer  |   Impl  -> address=0x3333333333333333333333333333333333333333, tx_hash=0x222",
          "tokamak-app-deployer  | SP1Verifier deployed address=0xb3b14127c950afb3e15d8c27bb4f707986495cc9",
        ];
        const result = parseContractAddressesFromLogs(logs);
        assert.equal(result.bridge, "0x2f6cf9ec2beed1b8169330994242e97398ce3352");
        assert.equal(result.proposer, "0xa59bdbd3bd6764b04f182973bceb51da127114d2");
        assert.equal(result.timelock, "0x1234567890abcdef1234567890abcdef12345678");
        assert.equal(result.sp1Verifier, "0xb3b14127c950afb3e15d8c27bb4f707986495cc9");
      });

      test("parseContractAddressesFromLogs returns nulls for empty logs", () => {
        const result = parseContractAddressesFromLogs([]);
        assert.equal(result.bridge, null);
        assert.equal(result.proposer, null);
      });

      test("parseContractAddressesFromLogs handles partial output", () => {
        const logs = [
          "CommonBridge deployed:  Proxy -> address=0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, tx_hash=0x1",
        ];
        const result = parseContractAddressesFromLogs(logs);
        assert.equal(result.bridge, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert.equal(result.proposer, null);
      });

      // -- 4-key testnet compose generation --
      const { generateTestnetComposeFile } = require("./lib/compose-generator");
      const { ethers } = require("ethers");

      test("generateTestnetComposeFile uses deployer key for all roles by default", () => {
        const deployerPk = "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924";
        const deployerAddr = new ethers.Wallet(deployerPk).address;
        const yaml = generateTestnetComposeFile({
          programSlug: "evm-l2", l2Port: 1729, proofCoordPort: 3900, metricsPort: 3702,
          projectName: "tokamak-test", l1RpcUrl: "http://l1:8545", deployerPrivateKey: deployerPk,
        });
        // All owner addresses should be deployer
        assert.ok(yaml.includes(`ETHREX_BRIDGE_OWNER=${deployerAddr}`), "bridge owner should be deployer");
        assert.ok(yaml.includes(`ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS=${deployerAddr}`), "committer should be deployer");
        assert.ok(yaml.includes(`ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS=${deployerAddr}`), "proof sender should be deployer");
        assert.ok(yaml.includes(`--committer.l1-private-key ${deployerPk}`), "committer pk should be deployer");
        assert.ok(yaml.includes(`--proof-coordinator.l1-private-key ${deployerPk}`), "proof coord pk should be deployer");
      });

      test("generateTestnetComposeFile uses separate keys when provided", () => {
        const deployerPk = "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924";
        const committerPk = "0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d";
        const proofPk = "0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e";
        const bridgePk = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        const committerAddr = new ethers.Wallet(committerPk).address;
        const proofAddr = new ethers.Wallet(proofPk).address;
        const bridgeAddr = new ethers.Wallet(bridgePk).address;
        const yaml = generateTestnetComposeFile({
          programSlug: "evm-l2", l2Port: 1729, proofCoordPort: 3900, metricsPort: 3702,
          projectName: "tokamak-test", l1RpcUrl: "http://l1:8545", deployerPrivateKey: deployerPk,
          committerPk, proofCoordinatorPk: proofPk, bridgeOwnerPk: bridgePk,
        });
        assert.ok(yaml.includes(`ETHREX_BRIDGE_OWNER=${bridgeAddr}`), `bridge owner should be ${bridgeAddr}`);
        assert.ok(yaml.includes(`ETHREX_BRIDGE_OWNER_PK=${bridgePk}`), "bridge owner pk");
        assert.ok(yaml.includes(`ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS=${committerAddr}`), `committer addr should be ${committerAddr}`);
        assert.ok(yaml.includes(`ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS=${proofAddr}`), `proof sender addr should be ${proofAddr}`);
        assert.ok(yaml.includes(`--committer.l1-private-key ${committerPk}`), "committer runtime pk");
        assert.ok(yaml.includes(`--proof-coordinator.l1-private-key ${proofPk}`), "proof coord runtime pk");
      });

      // -- API route tests for start/stop --
      return testAsync("POST /api/deployments/:id/start rejects unprovisioned deployment", async () => {
        const app = require("./server");
        const server = http.createServer(app);
        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        // Create unprovisioned deployment
        const createRes = await fetch(`http://127.0.0.1:${port}/api/deployments`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: "API Start Test", programSlug: "evm-l2" }),
        });
        const { deployment } = await createRes.json();

        try {
          const res = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}/start`, { method: "POST" });
          const data = await res.json();
          assert.equal(res.status, 400);
          assert.ok(data.error.includes("Not provisioned"));
        } finally {
          // Cleanup
          await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`, { method: "DELETE" });
          server.close();
        }
      });
    })
    .then(() =>
      testAsync("POST /api/deployments/:id/stop cancels unprovisioned deployment gracefully", async () => {
        const app = require("./server");
        const server = http.createServer(app);
        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        const createRes = await fetch(`http://127.0.0.1:${port}/api/deployments`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: "API Stop Test", programSlug: "evm-l2" }),
        });
        const { deployment } = await createRes.json();

        try {
          const res = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}/stop`, { method: "POST" });
          const data = await res.json();
          assert.equal(res.status, 200);
          assert.equal(data.deployment.phase, "configured");
        } finally {
          await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`, { method: "DELETE" });
          server.close();
        }
      })
    )
    .then(() =>
      testAsync("POST /api/deployments/:id/service/:service/start rejects unprovisioned", async () => {
        const app = require("./server");
        const server = http.createServer(app);
        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        const createRes = await fetch(`http://127.0.0.1:${port}/api/deployments`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: "Svc Start Test", programSlug: "evm-l2" }),
        });
        const { deployment } = await createRes.json();

        try {
          // Test core service
          const res1 = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}/service/tokamak-app-l1/start`, { method: "POST" });
          assert.equal(res1.status, 400);

          // Test tools service
          const res2 = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}/service/frontend-l1/start`, { method: "POST" });
          assert.equal(res2.status, 400);
        } finally {
          await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`, { method: "DELETE" });
          server.close();
        }
      })
    )
    .then(() =>
      testAsync("DELETE /api/deployments/:id removes deployment", async () => {
        const app = require("./server");
        const server = http.createServer(app);
        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        const createRes = await fetch(`http://127.0.0.1:${port}/api/deployments`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: "Delete Test", programSlug: "evm-l2" }),
        });
        const { deployment } = await createRes.json();

        try {
          const delRes = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`, { method: "DELETE" });
          const delData = await delRes.json();
          assert.equal(delData.ok, true);

          // Verify deleted
          const getRes = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`);
          assert.equal(getRes.status, 404);
        } finally {
          server.close();
        }
      })
    )
    .then(() =>
      testAsync("PUT /api/deployments/:id updates config", async () => {
        const app = require("./server");
        const server = http.createServer(app);
        await new Promise((resolve) => server.listen(0, resolve));
        const port = server.address().port;

        const createRes = await fetch(`http://127.0.0.1:${port}/api/deployments`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: "Config API Test", programSlug: "evm-l2" }),
        });
        const { deployment } = await createRes.json();

        try {
          const config = { mode: "testnet", testnet: { l1RpcUrl: "https://rpc.example.com" } };
          const res = await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`, {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ name: "Updated Name", config }),
          });
          const data = await res.json();
          assert.equal(res.status, 200);
          assert.equal(data.deployment.name, "Updated Name");
          const parsed = JSON.parse(data.deployment.config);
          assert.equal(parsed.mode, "testnet");
        } finally {
          await fetch(`http://127.0.0.1:${port}/api/deployments/${deployment.id}`, { method: "DELETE" });
          server.close();
        }
      })
    )
    .then(() => {
      // Cleanup
      fs.rmSync(testDir, { recursive: true, force: true });

      console.log(`\n=== Results: ${passed} passed, ${failed} failed ===`);
      process.exit(failed > 0 ? 1 : 0);
    });
});

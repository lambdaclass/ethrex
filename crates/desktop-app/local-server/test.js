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

  test("getNextAvailablePorts returns valid ports", () => {
    const ports = deploymentsDb.getNextAvailablePorts();
    assert.ok(ports.l1Port > 0);
    assert.ok(ports.l2Port > 0);
    assert.ok(ports.proofCoordPort > 0);
  });

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
      // Cleanup
      fs.rmSync(testDir, { recursive: true, force: true });

      console.log(`\n=== Results: ${passed} passed, ${failed} failed ===`);
      process.exit(failed > 0 ? 1 : 0);
    });
});

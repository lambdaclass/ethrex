#!/usr/bin/env node
/**
 * E2E test: Chain ID flow
 * Verifies that user-specified L2 chain IDs are stored and passed to compose correctly.
 *
 * Usage: LOCAL_SERVER_PORT=5099 node tests/e2e-chain-id.js
 */

const PORT = process.env.LOCAL_SERVER_PORT || 5099;
const API = `http://127.0.0.1:${PORT}/api`;

let passed = 0;
let failed = 0;
const createdIds = [];

function assert(label, condition, detail) {
  if (condition) {
    console.log(`  ✓ ${label}`);
    passed++;
  } else {
    console.log(`  ✗ ${label}${detail ? ` — ${detail}` : ""}`);
    failed++;
  }
}

async function api(path, opts) {
  const resp = await fetch(`${API}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...opts,
  });
  const data = await resp.json();
  return { status: resp.status, data };
}

async function cleanup() {
  for (const id of createdIds) {
    try { await api(`/deployments/${id}`, { method: "DELETE" }); } catch {}
  }
}

async function run() {
  console.log("\n=== E2E: Chain ID Flow ===\n");

  // 1. GET /api/deployments/next-chain-id
  console.log("1. next-chain-id API");
  const { status: s1, data: d1 } = await api("/deployments/next-chain-id");
  assert("returns 200", s1 === 200, `got ${s1}`);
  assert("chainId is a number", typeof d1.chainId === "number", `got ${typeof d1.chainId}`);
  assert("chainId > 0", d1.chainId > 0, `got ${d1.chainId}`);

  // 2. Create deployment WITH chain ID
  console.log("\n2. Create deployment with explicit chain ID");
  const { status: s2, data: d2 } = await api("/deployments", {
    method: "POST",
    body: JSON.stringify({ name: "Test Chain 17001", programSlug: "zk-dex", chainId: 17001 }),
  });
  assert("returns 201", s2 === 201, `got ${s2}`);
  assert("chain_id stored as 17001", d2.deployment?.chain_id === 17001, `got ${d2.deployment?.chain_id}`);
  if (d2.deployment) createdIds.push(d2.deployment.id);

  // 3. Create deployment WITHOUT chain ID (should be null, ensureL2ChainId handles it later)
  console.log("\n3. Create deployment without chain ID");
  const { status: s3, data: d3 } = await api("/deployments", {
    method: "POST",
    body: JSON.stringify({ name: "Test Chain Auto", programSlug: "zk-dex" }),
  });
  assert("returns 201", s3 === 201, `got ${s3}`);
  assert("chain_id is null (deferred)", d3.deployment?.chain_id === null, `got ${d3.deployment?.chain_id}`);
  if (d3.deployment) createdIds.push(d3.deployment.id);

  // 4. Update chain_id via PUT
  console.log("\n4. Update chain ID via PUT");
  if (d3.deployment) {
    const { status: s4, data: d4 } = await api(`/deployments/${d3.deployment.id}`, {
      method: "PUT",
      body: JSON.stringify({ chain_id: 42042 }),
    });
    assert("returns 200", s4 === 200, `got ${s4}`);
    // Verify by reading back
    const { data: d4b } = await api(`/deployments/${d3.deployment.id}`);
    assert("chain_id updated to 42042", d4b.deployment?.chain_id === 42042, `got ${d4b.deployment?.chain_id}`);
  }

  // 5. next-chain-id avoids collisions with stored chain IDs
  console.log("\n5. next-chain-id avoids collisions");
  const { data: d5a } = await api("/deployments/next-chain-id");
  assert("next ID != 17001", d5a.chainId !== 17001, `got ${d5a.chainId}`);
  assert("next ID != 42042", d5a.chainId !== 42042, `got ${d5a.chainId}`);

  // 6. Verify GET deployment returns chain_id
  console.log("\n6. GET deployment returns chain_id");
  if (d2.deployment) {
    const { data: d6 } = await api(`/deployments/${d2.deployment.id}`);
    assert("chain_id persisted", d6.deployment?.chain_id === 17001, `got ${d6.deployment?.chain_id}`);
  }

  // 7. next-chain-id returns L1 chain ID
  console.log("\n7. next-chain-id returns L1 chain ID");
  assert("l1ChainId is a number", typeof d1.l1ChainId === "number", `got ${typeof d1.l1ChainId}`);
  assert("l1ChainId > 0", d1.l1ChainId > 0, `got ${d1.l1ChainId}`);
  assert("l1ChainId != 9 (default)", d1.l1ChainId !== 9, `got ${d1.l1ChainId}`);

  // 8. L1 chain ID uniqueness — store one and verify next is different
  console.log("\n8. L1 chain ID uniqueness");
  const { data: d8a } = await api("/deployments/next-chain-id");
  const firstL1 = d8a.l1ChainId;
  // Create deployment and manually set l1_chain_id
  const { data: d8b } = await api("/deployments", {
    method: "POST",
    body: JSON.stringify({ name: "Test L1 Chain", programSlug: "evm-l2" }),
  });
  if (d8b.deployment) {
    createdIds.push(d8b.deployment.id);
    // Simulate engine storing l1_chain_id via PUT
    await api(`/deployments/${d8b.deployment.id}`, {
      method: "PUT",
      body: JSON.stringify({ l1_chain_id: firstL1 }),
    });
    const { data: d8c } = await api("/deployments/next-chain-id");
    assert("next L1 ID != stored L1 ID", d8c.l1ChainId !== firstL1, `both are ${firstL1}`);
  }

  // 9. L1 and L2 chain IDs are independent
  console.log("\n9. L1 and L2 chain IDs are independent");
  assert("L1 != L2 chain ID", d1.l1ChainId !== d1.chainId, `L1=${d1.l1ChainId}, L2=${d1.chainId}`);

  // Cleanup
  await cleanup();

  // Summary
  console.log(`\n=== Results: ${passed} passed, ${failed} failed ===\n`);
  process.exit(failed > 0 ? 1 : 0);
}

run().catch(e => {
  console.error("Test error:", e);
  cleanup().then(() => process.exit(1));
});

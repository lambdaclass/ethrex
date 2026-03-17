#!/usr/bin/env node
/**
 * Unit tests for db/deployments.js — getDeploymentByProposer
 */

const { describe, it, before } = require("node:test");
const assert = require("node:assert/strict");

// Force in-memory DB
process.env.DATABASE_URL = ":memory:";

delete require.cache[require.resolve("../db/db")];
delete require.cache[require.resolve("../db/deployments")];

const { getDb } = require("../db/db");
const { createDeployment, getDeploymentByProposer } = require("../db/deployments");

before(() => {
  // Ensure DB is initialized
  getDb();

  // Insert a test deployment directly (bypasses FK issues with createDeployment helper)
  const db = getDb();
  const programId = db.prepare("SELECT id FROM programs WHERE program_id = 'evm-l2'").get()?.id;
  if (!programId) throw new Error("Seed program not found");

  const deploymentId = "test-deploy-1";
  db.prepare(`
    INSERT INTO deployments (id, user_id, program_id, name, chain_id, rpc_url, status, phase,
      proposer_address, l1_chain_id, created_at)
    VALUES (?, 'system', ?, 'Test Deployment', 901, 'http://localhost:1234', 'active', 'running',
      ?, ?, ?)
  `).run(deploymentId, programId, "0xaabbccddee1122334455667788990011aabbccdd", 11155111, Date.now());
});

describe("getDeploymentByProposer", () => {
  it("finds deployment by proposer address and chain ID", () => {
    const result = getDeploymentByProposer("0xAABBCCDDEE1122334455667788990011AABBCCDD", 11155111);
    assert.ok(result, "should find the deployment");
    assert.ok(result.id);
  });

  it("returns undefined for unknown proposer", () => {
    const result = getDeploymentByProposer("0x0000000000000000000000000000000000000000", 11155111);
    assert.equal(result, undefined);
  });

  it("returns undefined for wrong chain ID", () => {
    const result = getDeploymentByProposer("0xaabbccddee1122334455667788990011aabbccdd", 1);
    assert.equal(result, undefined);
  });
});

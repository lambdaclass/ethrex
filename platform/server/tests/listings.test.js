#!/usr/bin/env node
/**
 * Unit tests for db/listings.js
 * Uses an in-memory SQLite database (DATABASE_URL=:memory:)
 */

const { describe, it, before, afterEach } = require("node:test");
const assert = require("node:assert/strict");
const path = require("path");
const fs = require("fs");

// Force in-memory DB before requiring any DB modules
process.env.DATABASE_URL = ":memory:";

// Clear module cache to ensure fresh DB
delete require.cache[require.resolve("../db/db")];
delete require.cache[require.resolve("../db/listings")];

const {
  listingId,
  upsertListing,
  getListingById,
  getListingByIdentityContract,
  getListings,
  getAllRepoFilePaths,
  getListingAddressesForChain,
  deleteListing,
  updateListingStatus,
  updateListingEnrichment,
} = require("../db/listings");

// ── Fixtures ──────────────────────────────────────────────────────

function makeMeta(overrides = {}) {
  return {
    l1ChainId: 11155111,
    l2ChainId: 901,
    stackType: "tokamak-appchain",
    identityContract: "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01",
    name: "Test Appchain",
    rollupType: "optimistic",
    status: "active",
    rpcUrl: "https://rpc.test.com",
    explorerUrl: "https://explorer.test.com",
    bridgeUrl: "https://bridge.test.com",
    dashboardUrl: null,
    nativeToken: { type: "eth", symbol: "ETH", decimals: 18, l1Address: null },
    l1Contracts: { bridge: "0x1111111111111111111111111111111111111111" },
    operator: { name: "Test Operator", website: "https://test.com", socialLinks: { twitter: "@test" } },
    description: "A test appchain",
    screenshots: ["https://img.test.com/1.png"],
    hashtags: ["defi", "test"],
    metadata: { signedBy: "0xSigner1234567890123456789012345678901234", signature: "0xsig" },
    ...overrides,
  };
}

// ── Tests ─────────────────────────────────────────────────────────

describe("listingId", () => {
  it("generates deterministic ID", () => {
    const id = listingId(11155111, "tokamak-appchain", "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01");
    assert.equal(id, "11155111-tokamak-appchain-0xabcdef0123456789abcdef0123456789abcdef01");
  });

  it("lowercases identity contract", () => {
    const id1 = listingId(1, "stack", "0xAAAABBBBCCCCDDDDEEEEFFFF0000111122223333");
    const id2 = listingId(1, "stack", "0xaaaabbbbccccddddeeee ffff0000111122223333".replace(/ /g, ""));
    // Both should produce lowercase
    assert.equal(id1, id1.toLowerCase().replace("0xaaaabbbbccccddddeeee ffff0000111122223333".replace(/ /g, ""), id1.split("-")[2]));
    assert.ok(id1.includes("0xaaaabbbbccccddddeeeeffff0000111122223333"));
  });
});

describe("upsertListing", () => {
  it("inserts a new listing", () => {
    const meta = makeMeta();
    upsertListing(meta, "tokamak-appchain-data/11155111/tokamak-appchain/0xAbCdEf0123456789AbCdEf0123456789AbCdEf01.json", "sha123");

    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);
    const row = getListingById(id);

    assert.ok(row, "listing should exist");
    assert.equal(row.name, "Test Appchain");
    assert.equal(row.l1_chain_id, 11155111);
    assert.equal(row.l2_chain_id, 901);
    assert.equal(row.stack_type, "tokamak-appchain");
    assert.equal(row.identity_contract, meta.identityContract.toLowerCase());
    assert.equal(row.rollup_type, "optimistic");
    assert.equal(row.status, "active");
    assert.equal(row.rpc_url, "https://rpc.test.com");
    assert.equal(row.native_token_symbol, "ETH");
    assert.equal(row.native_token_decimals, 18);
    assert.equal(row.operator_name, "Test Operator");
    assert.equal(row.repo_sha, "sha123");
    assert.equal(row.signed_by, "0xSigner1234567890123456789012345678901234");
    assert.ok(row.created_at > 0);
    assert.ok(row.synced_at > 0);
  });

  it("updates existing listing on conflict (upsert)", () => {
    const meta = makeMeta();
    upsertListing(meta, "path.json", "sha-v1");

    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);
    const before = getListingById(id);

    // Update with new data
    const updated = makeMeta({ name: "Updated Appchain", rpcUrl: "https://rpc2.test.com" });
    upsertListing(updated, "path.json", "sha-v2");

    const after = getListingById(id);
    assert.equal(after.name, "Updated Appchain");
    assert.equal(after.rpc_url, "https://rpc2.test.com");
    assert.equal(after.repo_sha, "sha-v2");
    assert.equal(after.created_at, before.created_at, "created_at should not change on update");
  });

  it("defaults status to active when not provided", () => {
    const meta = makeMeta({
      identityContract: "0x1111111111111111111111111111111111111111",
      status: undefined,
    });
    upsertListing(meta, null, null);
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);
    const row = getListingById(id);
    assert.equal(row.status, "active");
  });

  it("stores SHA atomically with the listing", () => {
    const meta = makeMeta({
      identityContract: "0x2222222222222222222222222222222222222222",
    });
    upsertListing(meta, "some/path.json", "abc123");
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);
    const row = getListingById(id);
    assert.equal(row.repo_sha, "abc123");
    assert.equal(row.repo_file_path, "some/path.json");
  });
});

describe("getListings", () => {
  it("returns active listings only", () => {
    const results = getListings();
    assert.ok(Array.isArray(results));
    for (const r of results) {
      assert.equal(r.status, "active");
    }
  });

  it("supports search filter", () => {
    const results = getListings({ search: "Updated" });
    assert.ok(results.length >= 1);
    assert.ok(results.some((r) => r.name.includes("Updated")));
  });

  it("supports stackType filter", () => {
    const results = getListings({ stackType: "tokamak-appchain" });
    for (const r of results) {
      assert.equal(r.stack_type, "tokamak-appchain");
    }
  });

  it("supports l1ChainId filter", () => {
    const results = getListings({ l1ChainId: "11155111" });
    for (const r of results) {
      assert.equal(r.l1_chain_id, 11155111);
    }
  });

  it("supports pagination", () => {
    const page1 = getListings({ limit: 1, offset: 0 });
    const page2 = getListings({ limit: 1, offset: 1 });
    assert.equal(page1.length, 1);
    if (page2.length > 0) {
      assert.notEqual(page1[0].id, page2[0].id);
    }
  });
});

describe("getListingByIdentityContract", () => {
  it("finds listing by contract address and chain ID", () => {
    const meta = makeMeta({
      identityContract: "0x3333333333333333333333333333333333333333",
    });
    upsertListing(meta, null, null);

    const result = getListingByIdentityContract("0x3333333333333333333333333333333333333333", 11155111);
    assert.ok(result);
    assert.equal(result.identity_contract, "0x3333333333333333333333333333333333333333");
  });

  it("returns undefined for non-existent contract", () => {
    const result = getListingByIdentityContract("0x9999999999999999999999999999999999999999", 11155111);
    assert.equal(result, undefined);
  });

  it("is case-insensitive on address", () => {
    const meta = makeMeta({
      identityContract: "0xAABBCCDDEEFF00112233445566778899AABBCCDD",
    });
    upsertListing(meta, null, null);

    const result = getListingByIdentityContract("0xaabbccddeeff00112233445566778899AABBCCDD", 11155111);
    assert.ok(result, "should find with different case");
  });
});

describe("getListingAddressesForChain", () => {
  it("returns identity contracts for given chain", () => {
    const addrs = getListingAddressesForChain(11155111);
    assert.ok(Array.isArray(addrs));
    assert.ok(addrs.length > 0);
    for (const addr of addrs) {
      assert.ok(addr.startsWith("0x"), "should be an address");
    }
  });

  it("returns empty for unknown chain", () => {
    const addrs = getListingAddressesForChain(99999);
    assert.deepEqual(addrs, []);
  });
});

describe("getAllRepoFilePaths", () => {
  it("returns listings with repo_file_path set", () => {
    const results = getAllRepoFilePaths();
    assert.ok(Array.isArray(results));
    for (const r of results) {
      assert.ok(r.id);
      assert.ok(r.repo_file_path);
    }
  });
});

describe("deleteListing", () => {
  it("removes a listing", () => {
    const meta = makeMeta({
      identityContract: "0x4444444444444444444444444444444444444444",
    });
    upsertListing(meta, null, null);
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);
    assert.ok(getListingById(id));

    deleteListing(id);
    assert.equal(getListingById(id), undefined);
  });
});

describe("updateListingStatus", () => {
  it("changes listing status", () => {
    const meta = makeMeta({
      identityContract: "0x5555555555555555555555555555555555555555",
    });
    upsertListing(meta, null, null);
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);

    updateListingStatus(id, "inactive");
    const row = getListingById(id);
    assert.equal(row.status, "inactive");
  });
});

describe("updateListingEnrichment", () => {
  it("updates allowed fields", () => {
    const meta = makeMeta({
      identityContract: "0x6666666666666666666666666666666666666666",
    });
    upsertListing(meta, null, null);
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);

    updateListingEnrichment(id, {
      description: "Enriched description",
      explorer_url: "https://enriched-explorer.com",
    });

    const row = getListingById(id);
    assert.equal(row.description, "Enriched description");
    assert.equal(row.explorer_url, "https://enriched-explorer.com");
  });

  it("ignores disallowed fields", () => {
    const meta = makeMeta({
      identityContract: "0x7777777777777777777777777777777777777777",
    });
    upsertListing(meta, null, null);
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);

    updateListingEnrichment(id, {
      name: "Hacked Name", // not in allowed list
      status: "deleted",   // not in allowed list
      description: "OK",   // allowed
    });

    const row = getListingById(id);
    assert.equal(row.name, "Test Appchain", "name should not be changed");
    assert.equal(row.status, "active", "status should not be changed");
    assert.equal(row.description, "OK");
  });

  it("does nothing when no allowed fields provided", () => {
    const meta = makeMeta({
      identityContract: "0x8888888888888888888888888888888888888888",
    });
    upsertListing(meta, null, null);
    const id = listingId(meta.l1ChainId, meta.stackType, meta.identityContract);

    // Should not throw
    updateListingEnrichment(id, { bad_field: "value" });
    const row = getListingById(id);
    assert.ok(row);
  });
});

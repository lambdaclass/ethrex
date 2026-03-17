#!/usr/bin/env node
/**
 * Unit tests for lib/metadata-push.js — buildMetadataJSON, getRepoFilePath
 * (Network-dependent functions like pushMetadataToRepo tested separately)
 */

const { describe, it } = require("node:test");
const assert = require("node:assert/strict");
const { buildMetadataJSON, getRepoFilePath } = require("../lib/metadata-push");

// ── Fixtures ──

function makeDeployment(overrides = {}) {
  return {
    name: "Test DEX Chain",
    chain_id: 901,
    rpc_url: "https://rpc.test-dex.com",
    status: "active",
    l1_chain_id: 11155111,
    proposer_address: "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01",
    bridge_address: "0x1111111111111111111111111111111111111111",
    program_slug: "zk-dex",
    description: "A test DEX appchain",
    explorer_url: "https://explorer.test-dex.com",
    dashboard_url: "https://dashboard.test-dex.com",
    screenshots: JSON.stringify(["ipfs://Qm1", "ipfs://Qm2"]),
    social_links: JSON.stringify({ website: "https://test.com", twitter: "@test" }),
    hashtags: JSON.stringify(["DeFi", "DEX"]),
    owner_wallet: "0xOwner1234567890123456789012345678901234",
    owner_name: "Test Team",
    native_token_type: "erc20",
    native_token_symbol: "TON",
    native_token_decimals: 18,
    native_token_l1_address: "0xTONAddress1234567890123456789012345678",
    ...overrides,
  };
}

// ── buildMetadataJSON ──

describe("buildMetadataJSON", () => {
  it("maps all deployment fields correctly", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);

    assert.equal(result.l1ChainId, 11155111);
    assert.equal(result.l2ChainId, 901);
    assert.equal(result.stackType, "zk-dex");
    assert.equal(result.identityContract, "0xabcdef0123456789abcdef0123456789abcdef01");
    assert.equal(result.name, "Test DEX Chain");
    assert.equal(result.description, "A test DEX appchain");
    assert.equal(result.rollupType, "zk");
    assert.equal(result.status, "active");
    assert.equal(result.rpcUrl, "https://rpc.test-dex.com");
    assert.equal(result.explorerUrl, "https://explorer.test-dex.com");
    assert.equal(result.dashboardUrl, "https://dashboard.test-dex.com");
  });

  it("maps native token from deployment fields", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);

    assert.equal(result.nativeToken.type, "erc20");
    assert.equal(result.nativeToken.symbol, "TON");
    assert.equal(result.nativeToken.decimals, 18);
    assert.equal(result.nativeToken.l1Address, "0xTONAddress1234567890123456789012345678");
  });

  it("defaults native token to ETH when not specified", () => {
    const deployment = makeDeployment({
      native_token_type: undefined,
      native_token_symbol: undefined,
      native_token_decimals: undefined,
    });
    const result = buildMetadataJSON(deployment);

    assert.equal(result.nativeToken.type, "eth");
    assert.equal(result.nativeToken.symbol, "ETH");
    assert.equal(result.nativeToken.decimals, 18);
  });

  it("maps L1 contracts correctly", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);

    assert.equal(result.l1Contracts.OnChainProposer, "0xabcdef0123456789abcdef0123456789abcdef01");
    assert.equal(result.l1Contracts.CommonBridge, "0x1111111111111111111111111111111111111111");
  });

  it("parses social_links JSON string", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);

    assert.equal(result.operator.website, "https://test.com");
    assert.equal(result.operator.socialLinks.twitter, "@test");
    assert.equal(result.supportResources.website, "https://test.com");
    assert.equal(result.supportResources.twitter, "@test");
  });

  it("parses screenshots JSON string", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);

    assert.deepEqual(result.screenshots, ["ipfs://Qm1", "ipfs://Qm2"]);
  });

  it("parses hashtags JSON string", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);

    assert.deepEqual(result.hashtags, ["DeFi", "DEX"]);
  });

  it("handles malformed social_links JSON gracefully", () => {
    const deployment = makeDeployment({ social_links: "invalid json{" });
    const result = buildMetadataJSON(deployment);

    assert.equal(result.operator.socialLinks, null);
    assert.equal(result.supportResources.website, null);
  });

  it("handles malformed screenshots JSON gracefully", () => {
    const deployment = makeDeployment({ screenshots: "not json" });
    const result = buildMetadataJSON(deployment);

    assert.deepEqual(result.screenshots, []);
  });

  it("handles null optional fields", () => {
    const deployment = makeDeployment({
      description: null,
      explorer_url: null,
      dashboard_url: null,
      social_links: null,
      screenshots: null,
      hashtags: null,
      owner_name: null,
    });
    const result = buildMetadataJSON(deployment);

    assert.equal(result.description, null);
    assert.equal(result.explorerUrl, null);
    assert.equal(result.dashboardUrl, null);
    assert.deepEqual(result.screenshots, []);
    assert.deepEqual(result.hashtags, []);
    assert.equal(result.operator.name, null);
  });

  it("maps inactive status correctly", () => {
    const deployment = makeDeployment({ status: "draft" });
    const result = buildMetadataJSON(deployment);
    assert.equal(result.status, "inactive");
  });

  it("sets metadata.signedBy from owner_wallet", () => {
    const deployment = makeDeployment();
    const result = buildMetadataJSON(deployment);
    assert.equal(result.metadata.signedBy, "0xOwner1234567890123456789012345678901234");
    assert.ok(result.metadata.updatedAt);
  });

  it("lowercases identityContract", () => {
    const deployment = makeDeployment({
      proposer_address: "0xAABBCCDDEEFF00112233445566778899AABBCCDD",
    });
    const result = buildMetadataJSON(deployment);
    assert.equal(result.identityContract, "0xaabbccddeeff00112233445566778899aabbccdd");
  });

  it("defaults l1ChainId to 1 when not set", () => {
    const deployment = makeDeployment({ l1_chain_id: null });
    const result = buildMetadataJSON(deployment);
    assert.equal(result.l1ChainId, 1);
  });

  it("defaults program_slug to tokamak-appchain", () => {
    const deployment = makeDeployment({ program_slug: undefined });
    const result = buildMetadataJSON(deployment);
    assert.equal(result.stackType, "tokamak-appchain");
  });
});

// ── getRepoFilePath ──

describe("getRepoFilePath", () => {
  it("generates correct path from deployment", () => {
    const deployment = makeDeployment();
    const path = getRepoFilePath(deployment);
    assert.equal(
      path,
      "tokamak-appchain-data/11155111/zk-dex/0xabcdef0123456789abcdef0123456789abcdef01.json"
    );
  });

  it("uses l1_chain_id in path", () => {
    const deployment = makeDeployment({ l1_chain_id: 1 });
    const path = getRepoFilePath(deployment);
    assert.ok(path.startsWith("tokamak-appchain-data/1/"));
  });

  it("uses program_slug as stack type in path", () => {
    const deployment = makeDeployment({ program_slug: "evm-l2" });
    const path = getRepoFilePath(deployment);
    assert.ok(path.includes("/evm-l2/"));
  });

  it("lowercases proposer address in path", () => {
    const deployment = makeDeployment({
      proposer_address: "0xAABBCCDDEEFF00112233445566778899AABBCCDD",
    });
    const path = getRepoFilePath(deployment);
    assert.ok(path.includes("0xaabbccddeeff00112233445566778899aabbccdd.json"));
  });

  it("returns null when proposer_address is missing", () => {
    const deployment = makeDeployment({ proposer_address: null });
    const path = getRepoFilePath(deployment);
    assert.equal(path, null);
  });

  it("returns null when proposer_address is empty string", () => {
    const deployment = makeDeployment({ proposer_address: "" });
    const path = getRepoFilePath(deployment);
    assert.equal(path, null);
  });

  it("returns null when proposer_address has no 0x prefix", () => {
    const deployment = makeDeployment({ proposer_address: "aabbccdd" });
    const path = getRepoFilePath(deployment);
    assert.equal(path, null);
  });

  it("defaults l1_chain_id to 1 when null", () => {
    const deployment = makeDeployment({ l1_chain_id: null });
    const path = getRepoFilePath(deployment);
    assert.ok(path.startsWith("tokamak-appchain-data/1/"));
  });

  it("defaults program_slug to tokamak-appchain", () => {
    const deployment = makeDeployment({ program_slug: undefined });
    const path = getRepoFilePath(deployment);
    assert.ok(path.includes("/tokamak-appchain/"));
  });
});

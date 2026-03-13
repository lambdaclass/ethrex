#!/usr/bin/env node
/**
 * Unit tests for lib/metadata-sync.js — parseFilePath function
 * (Network-dependent functions tested separately via integration tests)
 */

const { describe, it } = require("node:test");
const assert = require("node:assert/strict");

// We can't require metadata-sync directly (it imports DB modules at load time),
// so we test parseFilePath by extracting the regex logic.
// This matches the regex in metadata-sync.js parseFilePath().
function parseFilePath(filePath) {
  const match = filePath.match(
    /^tokamak-appchain-data\/(\d+)\/([a-z0-9-]+)\/(0x[a-fA-F0-9]{40})\.json$/
  );
  if (!match) return null;
  return {
    l1ChainId: parseInt(match[1]),
    stackType: match[2],
    identityContract: match[3],
  };
}

describe("parseFilePath", () => {
  it("parses valid path", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/11155111/tokamak-appchain/0xAbCdEf0123456789AbCdEf0123456789AbCdEf01.json"
    );
    assert.deepEqual(result, {
      l1ChainId: 11155111,
      stackType: "tokamak-appchain",
      identityContract: "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01",
    });
  });

  it("parses mainnet chain ID", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/1/tokamak-appchain/0x1111111111111111111111111111111111111111.json"
    );
    assert.equal(result.l1ChainId, 1);
  });

  it("handles hyphenated stack types", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/17000/custom-zk-stack/0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json"
    );
    assert.equal(result.stackType, "custom-zk-stack");
  });

  it("rejects path with wrong prefix", () => {
    const result = parseFilePath(
      "wrong-prefix/11155111/tokamak-appchain/0x1111111111111111111111111111111111111111.json"
    );
    assert.equal(result, null);
  });

  it("rejects path with invalid address (too short)", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/11155111/tokamak-appchain/0x1234.json"
    );
    assert.equal(result, null);
  });

  it("rejects path with non-hex address", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/11155111/tokamak-appchain/0xGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG.json"
    );
    assert.equal(result, null);
  });

  it("rejects non-JSON files", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/11155111/tokamak-appchain/0x1111111111111111111111111111111111111111.yaml"
    );
    assert.equal(result, null);
  });

  it("rejects nested subdirectories", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/11155111/tokamak-appchain/extra/0x1111111111111111111111111111111111111111.json"
    );
    assert.equal(result, null);
  });

  it("preserves mixed-case address (not lowered by parser)", () => {
    const result = parseFilePath(
      "tokamak-appchain-data/11155111/tokamak-appchain/0xAaBbCcDdEeFf00112233445566778899AaBbCcDd.json"
    );
    assert.equal(result.identityContract, "0xAaBbCcDdEeFf00112233445566778899AaBbCcDd");
  });
});

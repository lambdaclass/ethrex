require("@nomicfoundation/hardhat-ethers");
require("@nomicfoundation/hardhat-chai-matchers");
require("@openzeppelin/hardhat-upgrades");

const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "../..");
const DEFAULT_KEYS_PATH = path.join(
  ROOT,
  "fixtures/keys/private_keys_tests.txt"
);

function loadKeys(filePath) {
  try {
    const data = fs.readFileSync(filePath, "utf8");
    return data
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
  } catch (err) {
    return [];
  }
}

function networkConfig(url, chainId, accounts) {
  const config = { url, chainId };
  if (accounts.length > 0) {
    config.accounts = accounts;
  }
  return config;
}

const l1KeysPath =
  process.env.ETHREX_L1_KEYS_FILE ||
  process.env.ETHREX_KEYS_FILE ||
  DEFAULT_KEYS_PATH;
const l2KeysPath =
  process.env.ETHREX_L2_KEYS_FILE ||
  process.env.ETHREX_KEYS_FILE ||
  DEFAULT_KEYS_PATH;

const l1Accounts = loadKeys(l1KeysPath);
const l2Accounts = loadKeys(l2KeysPath);

module.exports = {
  paths: {
    root: ROOT,
    sources: path.join(ROOT, "crates/l2/contracts/src"),
    tests: path.join(__dirname, "test"),
    cache: path.join(__dirname, "cache"),
    artifacts: path.join(__dirname, "artifacts")
  },
  solidity: {
    version: "0.8.31",
    settings: {
      evmVersion: "cancun",
      viaIR: true,
      optimizer: {
        enabled: true,
        runs: 999999
      },
      metadata: {
        bytecodeHash: "none"
      }
    }
  },
  networks: {
    ethrexL1: networkConfig(
      process.env.ETHREX_L1_RPC_URL || "http://127.0.0.1:8545",
      Number(process.env.ETHREX_L1_CHAIN_ID || "9"),
      l1Accounts
    ),
    ethrexL2: networkConfig(
      process.env.ETHREX_L2_RPC_URL || "http://127.0.0.1:1729",
      Number(process.env.ETHREX_L2_CHAIN_ID || "65536999"),
      l2Accounts
    )
  }
};

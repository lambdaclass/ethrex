require("@nomicfoundation/hardhat-ethers");
require("@nomicfoundation/hardhat-chai-matchers");
require("@openzeppelin/hardhat-upgrades");

const path = require("path");

const ROOT = path.join(__dirname, "../..");

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
  }
};

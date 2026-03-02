require("@nomicfoundation/hardhat-ethers");
require("@nomicfoundation/hardhat-chai-matchers");
require("@openzeppelin/hardhat-upgrades");

const fs = require("fs");
const path = require("path");
const { subtask } = require("hardhat/config");
const {
  TASK_COMPILE_SOLIDITY_GET_SOURCE_PATHS
} = require("hardhat/builtin-tasks/task-names");

const ROOT = path.join(__dirname, "../..");
const UPGRADE_FIXTURES_DIR = path.join(
  ROOT,
  "fixtures/contracts/upgradeability"
);

function collectSolFiles(dir, files = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      collectSolFiles(fullPath, files);
    } else if (entry.isFile() && entry.name.endsWith(".sol")) {
      files.push(fullPath);
    }
  }
  return files;
}

subtask(TASK_COMPILE_SOLIDITY_GET_SOURCE_PATHS).setAction(
  async (taskArgs, hre, runSuper) => {
    const sources = await runSuper(taskArgs);
    if (!fs.existsSync(UPGRADE_FIXTURES_DIR)) {
      return sources;
    }

    const extraSources = collectSolFiles(UPGRADE_FIXTURES_DIR);
    if (extraSources.length === 0) {
      return sources;
    }

    return Array.from(new Set([...sources, ...extraSources]));
  }
);

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

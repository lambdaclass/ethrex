/**
 * Etherscan Contract Verification Module
 *
 * Verifies deployed contracts on Etherscan for testnet/mainnet deployments.
 * Uses forge flatten to prepare source code and submits via Etherscan API.
 *
 * Verification flow:
 * 1. Flatten Solidity source using `forge flatten`
 * 2. Submit source to Etherscan verify API
 * 3. Poll for verification status
 * 4. Submit proxy verification for upgradeable contracts
 */

const { execSync } = require("child_process");
const path = require("path");
const fs = require("fs");

// Etherscan API V2: single endpoint with chainid parameter
// See: https://docs.etherscan.io/v2-migration
const ETHERSCAN_V2_BASE = "https://api.etherscan.io/v2/api";
const SUPPORTED_CHAIN_IDS = new Set([1, 11155111, 17000]);

function getApiUrl(chainId) {
  return `${ETHERSCAN_V2_BASE}?chainid=${chainId}`;
}

// Contract source paths (relative to ethrex root)
const CONTRACT_SOURCES = {
  CommonBridge: "crates/l2/contracts/src/l1/CommonBridge.sol",
  OnChainProposer: "crates/l2/contracts/src/l1/OnChainProposer.sol",
  Timelock: "crates/l2/contracts/src/l1/Timelock.sol",
  GuestProgramRegistry: "crates/l2/contracts/src/l1/GuestProgramRegistry.sol",
  Router: "crates/l2/contracts/src/l1/Router.sol",
};

// Contract solc settings (must match deployer compilation)
const COMPILER_VERSION = "v0.8.31+commit.46dfe0e0";
const OPTIMIZATION = true;
const OPTIMIZATION_RUNS = 999999;

/**
 * Get the ethrex project root directory
 */
function getEthrexRoot() {
  // local-server is at crates/desktop-app/local-server
  return path.resolve(__dirname, "..", "..", "..", "..");
}

/**
 * Flatten a Solidity source file using `forge flatten`.
 * Requires Foundry to be installed; throws if forge is not available.
 */
function flattenSource(contractName) {
  const ethrexRoot = getEthrexRoot();
  const contractsDir = path.join(ethrexRoot, "crates", "l2", "contracts");
  const srcPath = CONTRACT_SOURCES[contractName];
  if (!srcPath) throw new Error(`Unknown contract: ${contractName}`);

  const fullPath = path.join(ethrexRoot, srcPath);
  if (!fs.existsSync(fullPath)) {
    throw new Error(`Contract source not found: ${fullPath}`);
  }

  try {
    // Use forge flatten from the contracts directory (where foundry.toml lives)
    const flattened = execSync(
      `forge flatten "${fullPath}"`,
      { cwd: contractsDir, stdio: "pipe", timeout: 30000 }
    ).toString();
    return flattened;
  } catch (err) {
    throw new Error(`forge flatten failed for ${contractName}: ${err.message}. Is Foundry installed?`);
  }
}

/**
 * Submit contract source code to Etherscan for verification.
 * Returns the verification GUID for status polling.
 */
async function submitVerification({
  chainId,
  contractAddress,
  contractName,
  sourceCode,
  constructorArgs = "",
  apiKey,
}) {
  if (!SUPPORTED_CHAIN_IDS.has(chainId)) throw new Error(`Unsupported chain ID for Etherscan: ${chainId}`);
  if (!apiKey) throw new Error("ETHERSCAN_API_KEY is required for contract verification");

  const apiUrl = getApiUrl(chainId);
  const srcFile = CONTRACT_SOURCES[contractName];
  if (!srcFile) throw new Error(`Unknown contract name: ${contractName}`);
  const contractFullName = `${path.basename(srcFile)}:${contractName}`;

  const params = new URLSearchParams();
  params.append("apikey", apiKey);
  params.append("module", "contract");
  params.append("action", "verifysourcecode");
  params.append("contractaddress", contractAddress);
  params.append("sourceCode", sourceCode);
  params.append("codeformat", "solidity-single-file");
  params.append("contractname", contractFullName);
  params.append("compilerversion", COMPILER_VERSION);
  params.append("optimizationUsed", OPTIMIZATION ? "1" : "0");
  params.append("runs", String(OPTIMIZATION_RUNS));
  params.append("evmversion", "cancun");
  params.append("licenseType", "3"); // MIT
  if (constructorArgs) {
    params.append("constructorArguements", constructorArgs); // Etherscan typo is intentional
  }

  const response = await fetch(apiUrl, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: params.toString(),
  });

  const data = await response.json();
  if (data.status === "1" && data.result) {
    return data.result; // GUID for polling
  }
  throw new Error(`Etherscan verification submit failed: ${data.result || data.message}`);
}

/**
 * Poll Etherscan for verification status.
 * Returns true if verified, false if failed.
 */
async function checkVerificationStatus({ chainId, guid, apiKey, maxRetries = 10 }) {
  const apiUrl = getApiUrl(chainId);

  for (let i = 0; i < maxRetries; i++) {
    await new Promise(r => setTimeout(r, 5000)); // Wait 5s between checks

    const url = `${apiUrl}&module=contract&action=checkverifystatus&guid=${guid}&apikey=${apiKey}`;
    const response = await fetch(url);
    const data = await response.json();

    if (data.result === "Pass - Verified") return true;
    if (data.result === "Fail - Unable to verify") return false;
    if (data.result === "Already Verified") return true;
    // "Pending in queue" — keep waiting
  }
  return false;
}

/**
 * Submit proxy verification for upgradeable contracts (UUPS proxy).
 */
async function verifyProxy({ chainId, proxyAddress, apiKey }) {
  if (!SUPPORTED_CHAIN_IDS.has(chainId)) return;
  const apiUrl = getApiUrl(chainId);

  const params = new URLSearchParams();
  params.append("apikey", apiKey);
  params.append("module", "contract");
  params.append("action", "verifyproxycontract");
  params.append("address", proxyAddress);

  try {
    const response = await fetch(apiUrl, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: params.toString(),
    });
    const data = await response.json();
    if (data.status === "1") {
      // Poll for proxy verification
      const proxyGuid = data.result;
      for (let i = 0; i < 5; i++) {
        await new Promise(r => setTimeout(r, 3000));
        const checkUrl = `${apiUrl}&module=contract&action=checkproxyverification&guid=${proxyGuid}&apikey=${apiKey}`;
        const checkRes = await fetch(checkUrl);
        const checkData = await checkRes.json();
        if (checkData.result && !checkData.result.includes("Pending")) return true;
      }
    }
  } catch {
    // Proxy verification is best-effort
  }
  return false;
}

/**
 * Verify a single contract on Etherscan.
 * Handles flattening, submission, polling, and proxy verification.
 *
 * @param {Object} opts
 * @param {number} opts.chainId - L1 chain ID
 * @param {string} opts.contractName - e.g. "CommonBridge"
 * @param {string} opts.contractAddress - deployed proxy/contract address
 * @param {string} opts.apiKey - Etherscan API key
 * @param {boolean} opts.isProxy - whether this is a proxy contract (needs proxy verification)
 * @param {Function} opts.log - logging function
 * @returns {{ verified: boolean, error?: string }}
 */
async function verifyContract({ chainId, contractName, contractAddress, apiKey, isProxy = true, log = console.log }) {
  try {
    log(`[etherscan] Flattening ${contractName}...`);
    const sourceCode = flattenSource(contractName);

    log(`[etherscan] Submitting ${contractName} (${contractAddress}) for verification...`);
    const guid = await submitVerification({
      chainId,
      contractAddress,
      contractName,
      sourceCode,
      apiKey,
    });

    log(`[etherscan] Verification submitted (GUID: ${guid}), polling status...`);
    const verified = await checkVerificationStatus({ chainId, guid, apiKey });

    if (verified) {
      log(`[etherscan] ${contractName} verified successfully!`);
      if (isProxy) {
        log(`[etherscan] Submitting proxy verification for ${contractName}...`);
        await verifyProxy({ chainId, proxyAddress: contractAddress, apiKey });
        log(`[etherscan] Proxy verification submitted for ${contractName}`);
      }
      return { verified: true };
    } else {
      const msg = `${contractName} verification failed`;
      log(`[etherscan] ${msg}`);
      return { verified: false, error: msg };
    }
  } catch (err) {
    log(`[etherscan] ${contractName} verification error: ${err.message}`);
    return { verified: false, error: err.message };
  }
}

/**
 * Verify all deployed contracts for a deployment.
 * Only runs for testnet/mainnet (checks chainId).
 *
 * @param {Object} opts
 * @param {number} opts.chainId - L1 chain ID (1=mainnet, 11155111=sepolia, 17000=holesky)
 * @param {Object} opts.contracts - { bridge, proposer, timelock, sp1Verifier, guestProgramRegistry }
 * @param {string} opts.apiKey - Etherscan API key
 * @param {Function} opts.log - logging function
 * @returns {Object} verification status per contract
 */
async function verifyAllContracts({ chainId, contracts, apiKey, log = console.log }) {
  if (!SUPPORTED_CHAIN_IDS.has(chainId)) {
    log(`[etherscan] Chain ID ${chainId} not supported for Etherscan verification, skipping`);
    return {};
  }
  if (!apiKey) {
    log(`[etherscan] No ETHERSCAN_API_KEY provided, skipping verification`);
    return {};
  }

  const results = {};

  // Verify proxy contracts (CommonBridge, OnChainProposer, Timelock, GuestProgramRegistry)
  const proxyContracts = [
    { name: "CommonBridge", address: contracts.bridge },
    { name: "OnChainProposer", address: contracts.proposer },
    { name: "Timelock", address: contracts.timelock },
    { name: "GuestProgramRegistry", address: contracts.guestProgramRegistry },
  ];

  for (const { name, address } of proxyContracts) {
    if (!address) continue;
    results[name] = await verifyContract({ chainId, contractName: name, contractAddress: address, apiKey, isProxy: true, log });
    // Rate limit: Etherscan free tier allows ~1 req/5s
    await new Promise(r => setTimeout(r, 5000));
  }

  // SP1Verifier is not a proxy and uses create2 deployment — skip for now
  // (third-party contract, Etherscan may auto-match it)

  return results;
}

module.exports = {
  verifyContract,
  verifyAllContracts,
  flattenSource,
  SUPPORTED_CHAIN_IDS,
};

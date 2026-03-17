/**
 * Appchain Registry — Submit signed metadata to GitHub metadata repository via PR.
 *
 * POST /api/appchain-registry/submit — No auth required (signature = auth)
 * GET  /api/appchain-registry/status/:prNumber — Check PR status
 */

const express = require("express");
const { ethers } = require("ethers");
const {
  getBranchSHA,
  createBranch,
  createOrUpdateFile,
  createPullRequest,
  getPullRequest,
  findOpenPR,
  updatePullRequest,
  getFileContent,
} = require("../lib/github-pr");

const router = express.Router();

// RPC URLs by L1 chain ID
// RPC URLs by L1 chain ID — custom RPCs via env vars: L1_RPC_{chainId}=url
const L1_RPC_URLS = {
  1: process.env.MAINNET_RPC_URL || "https://ethereum-rpc.publicnode.com",
  11155111: process.env.SEPOLIA_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com",
  17000: process.env.HOLESKY_RPC_URL || "https://ethereum-holesky-rpc.publicnode.com",
};

// Allow custom L1 RPCs via env: L1_RPC_9=http://54.180.160.159:8545
for (const [key, val] of Object.entries(process.env)) {
  const m = key.match(/^L1_RPC_(\d+)$/);
  if (m && val) L1_RPC_URLS[parseInt(m[1])] = val;
}

// Timelock ABI (hasRole only)
const TIMELOCK_ABI = [
  "function hasRole(bytes32 role, address account) view returns (bool)",
];
const SECURITY_COUNCIL_ROLE = ethers.id("SECURITY_COUNCIL");

// Stack type -> identity contract field mapping
const IDENTITY_CONTRACT_FIELD = {
  "tokamak-appchain": "Timelock",
};

// Rate limit for submissions: 5 per hour per IP
const submitRateLimit = new Map();
const SUBMIT_RATE_WINDOW = 60 * 60 * 1000; // 1 hour
const SUBMIT_RATE_MAX = 5;

function checkSubmitRateLimit(ip) {
  const now = Date.now();
  // Evict stale entries periodically (every 100 checks)
  if (submitRateLimit.size > 100) {
    for (const [key, rec] of submitRateLimit) {
      if (now - rec.windowStart > SUBMIT_RATE_WINDOW) submitRateLimit.delete(key);
    }
  }
  const record = submitRateLimit.get(ip);
  if (!record || now - record.windowStart > SUBMIT_RATE_WINDOW) {
    submitRateLimit.set(ip, { windowStart: now, count: 1 });
    return true;
  }
  record.count++;
  return record.count <= SUBMIT_RATE_MAX;
}

/**
 * Build the signing message (must match signature-validator.ts format exactly).
 */
function buildSigningMessage(metadata, operation) {
  const identityField = IDENTITY_CONTRACT_FIELD[metadata.stackType];
  if (!identityField) throw new Error(`Unknown stackType: ${metadata.stackType}`);

  const identityAddress = metadata.l1Contracts[identityField];
  if (!identityAddress) throw new Error(`Missing l1Contracts.${identityField}`);

  let timestamp;
  if (operation === "register") {
    timestamp = Math.floor(new Date(metadata.createdAt).getTime() / 1000);
  } else {
    timestamp = Math.floor(new Date(metadata.lastUpdated).getTime() / 1000);
  }

  return [
    "Tokamak Appchain Registry",
    `L1 Chain ID: ${metadata.l1ChainId}`,
    `L2 Chain ID: ${metadata.l2ChainId}`,
    `Stack: ${metadata.stackType}`,
    `Operation: ${operation}`,
    `Contract: ${identityAddress.toLowerCase()}`,
    `Timestamp: ${timestamp}`,
  ].join("\n");
}

/**
 * Validate metadata structure (basic required fields check).
 */
function validateMetadataStructure(metadata) {
  const errors = [];
  if (!metadata.l1ChainId) errors.push("Missing l1ChainId");
  if (!metadata.l2ChainId) errors.push("Missing l2ChainId");
  if (!metadata.name) errors.push("Missing name");
  if (!metadata.stackType) errors.push("Missing stackType");
  if (!metadata.rollupType) errors.push("Missing rollupType");
  if (!metadata.rpcUrl) errors.push("Missing rpcUrl");
  if (!metadata.status) errors.push("Missing status");
  if (!metadata.createdAt) errors.push("Missing createdAt");
  if (!metadata.lastUpdated) errors.push("Missing lastUpdated");
  if (!metadata.l1Contracts) errors.push("Missing l1Contracts");
  if (!metadata.operator?.address) errors.push("Missing operator.address");
  if (!metadata.metadata?.signature) errors.push("Missing metadata.signature");
  if (!metadata.metadata?.signedBy) errors.push("Missing metadata.signedBy");

  const identityField = IDENTITY_CONTRACT_FIELD[metadata.stackType];
  if (identityField && metadata.l1Contracts && !metadata.l1Contracts[identityField]) {
    errors.push(`Missing l1Contracts.${identityField}`);
  }
  if (metadata.l1Contracts && !metadata.l1Contracts.OnChainProposer) {
    errors.push("Missing l1Contracts.OnChainProposer");
  }

  // Signature format: 0x + 130 hex chars
  if (metadata.metadata?.signature && !/^0x[a-fA-F0-9]{130}$/.test(metadata.metadata.signature)) {
    errors.push("Invalid signature format");
  }

  return errors;
}

/**
 * GET /api/appchain-registry/check/:l1ChainId/:stackType/:identityAddress
 * Check if metadata file already exists in the registry (on main branch).
 * Returns { exists, createdAt? } so the client can decide register vs update.
 */
router.get("/check/:l1ChainId/:stackType/:identityAddress", async (req, res) => {
  try {
    const { l1ChainId, stackType, identityAddress } = req.params;
    const filePath = `tokamak-appchain-data/${l1ChainId}/${stackType}/${identityAddress.toLowerCase()}.json`;
    const existing = await getFileContent(filePath, "main");
    if (existing) {
      return res.json({
        exists: true,
        createdAt: existing.createdAt || null,
        // Return immutable fields so client can preserve them on update
        l2ChainId: existing.l2ChainId || null,
        nativeToken: existing.nativeToken || null,
      });
    }
    return res.json({ exists: false });
  } catch (e) {
    // If GitHub API fails (e.g., no token), assume not exists
    console.warn("[appchain-registry] check failed:", e.message);
    return res.json({ exists: false });
  }
});

/**
 * POST /api/appchain-registry/submit
 */
router.post("/submit", async (req, res) => {
  try {
    const ip = req.ip || req.connection.remoteAddress;
    if (!checkSubmitRateLimit(ip)) {
      return res.status(429).json({
        success: false,
        error: "Rate limit exceeded. Max 5 submissions per hour.",
        code: "RATE_LIMITED",
      });
    }

    const { metadata, operation = "register" } = req.body;
    if (!metadata) {
      return res.status(400).json({ success: false, error: "Missing metadata", code: "INVALID_METADATA" });
    }

    // 1. Validate structure
    const structErrors = validateMetadataStructure(metadata);
    if (structErrors.length > 0) {
      return res.status(400).json({
        success: false,
        error: `Metadata validation failed: ${structErrors.join(", ")}`,
        code: "INVALID_METADATA",
      });
    }

    // 2. Verify signature — recover signer address
    let recoveredAddress;
    try {
      const message = buildSigningMessage(metadata, operation);
      recoveredAddress = ethers.verifyMessage(message, metadata.metadata.signature);
    } catch (e) {
      return res.status(400).json({
        success: false,
        error: `Signature verification failed: ${e.message}`,
        code: "INVALID_SIGNATURE",
      });
    }

    if (recoveredAddress.toLowerCase() !== metadata.metadata.signedBy.toLowerCase()) {
      return res.status(400).json({
        success: false,
        error: `Recovered signer ${recoveredAddress} does not match signedBy ${metadata.metadata.signedBy}`,
        code: "INVALID_SIGNATURE",
      });
    }

    // 3. Check timestamp (24h expiry)
    const ts = operation === "register"
      ? Math.floor(new Date(metadata.createdAt).getTime() / 1000)
      : Math.floor(new Date(metadata.lastUpdated).getTime() / 1000);
    if (!Number.isFinite(ts)) {
      return res.status(400).json({ success: false, error: "Invalid timestamp in createdAt/lastUpdated", code: "INVALID_METADATA" });
    }
    const now = Math.floor(Date.now() / 1000);
    if (ts > now + 300) {
      return res.status(400).json({ success: false, error: "Timestamp is in the future", code: "SIGNATURE_EXPIRED" });
    }
    if (now - ts > 86400) {
      return res.status(400).json({ success: false, error: "Signature expired (>24h)", code: "SIGNATURE_EXPIRED" });
    }

    // 4. On-chain ownership check: Timelock.hasRole(SECURITY_COUNCIL, signer)
    const rpcUrl = L1_RPC_URLS[metadata.l1ChainId] || metadata.l1RpcUrl;
    if (!rpcUrl) {
      return res.status(400).json({
        success: false,
        error: `No L1 RPC available for chain ID ${metadata.l1ChainId}. Include l1RpcUrl in metadata.`,
        code: "INVALID_METADATA",
      });
    }

    const identityField = IDENTITY_CONTRACT_FIELD[metadata.stackType];
    const timelockAddress = metadata.l1Contracts[identityField];

    try {
      const provider = new ethers.JsonRpcProvider(rpcUrl);
      const timelock = new ethers.Contract(timelockAddress, TIMELOCK_ABI, provider);
      const hasRole = await timelock.hasRole(SECURITY_COUNCIL_ROLE, recoveredAddress);

      if (!hasRole) {
        return res.status(403).json({
          success: false,
          error: `Signer ${recoveredAddress} does not have SECURITY_COUNCIL role on Timelock ${timelockAddress}`,
          code: "OWNERSHIP_CHECK_FAILED",
        });
      }
    } catch (e) {
      return res.status(502).json({
        success: false,
        error: `On-chain verification failed: ${e.message}`,
        code: "OWNERSHIP_CHECK_FAILED",
      });
    }

    // 5. Determine file path
    const filePath = `tokamak-appchain-data/${metadata.l1ChainId}/${metadata.stackType}/${timelockAddress.toLowerCase()}.json`;

    // 6. Check for existing open PR — update it instead of rejecting
    const existingPR = await findOpenPR(filePath);

    const fileContent = JSON.stringify(metadata, null, 2) + "\n";
    const commitMsg = operation === "register"
      ? `feat: register ${metadata.name} (${metadata.stackType})`
      : `feat: update ${metadata.name} (${metadata.stackType})`;

    const prTag = operation === "update" ? "Update" : "Appchain";
    const prTitle = `[${prTag}] ${metadata.l1ChainId}/${metadata.stackType} ${timelockAddress.toLowerCase()} - ${metadata.name}`;
    const prBody = [
      `## ${operation === "register" ? "New" : "Update"} Appchain: ${metadata.name}`,
      "",
      `| Field | Value |`,
      `|---|---|`,
      `| L1 Chain ID | ${metadata.l1ChainId} |`,
      `| L2 Chain ID | ${metadata.l2ChainId} |`,
      `| Stack | ${metadata.stackType} |`,
      `| Rollup Type | ${metadata.rollupType} |`,
      `| Timelock | \`${timelockAddress}\` |`,
      `| OnChainProposer | \`${metadata.l1Contracts.OnChainProposer}\` |`,
      `| Operator | \`${metadata.operator.address}\` |`,
      `| Signed by | \`${metadata.metadata.signedBy}\` |`,
      "",
      "---",
      "*Submitted via Tokamak Appchain Messenger*",
    ].join("\n");

    if (existingPR && existingPR.headBranch) {
      // Update the existing PR's branch with new metadata + title/body
      await createOrUpdateFile(filePath, fileContent, existingPR.headBranch, commitMsg);
      await updatePullRequest(existingPR.prNumber, { title: prTitle, body: prBody });

      console.log(`[appchain-registry] PR #${existingPR.prNumber} updated for ${metadata.name}`);

      return res.json({
        success: true,
        updated: true,
        prUrl: existingPR.prUrl,
        prNumber: existingPR.prNumber,
        filePath,
      });
    }

    // 7. Create new GitHub PR
    const branchName = `appchain-registry/${metadata.l1ChainId}/${timelockAddress.toLowerCase().slice(0, 10)}/${ts}`;

    const mainSha = await getBranchSHA("main");
    await createBranch(branchName, mainSha);
    await createOrUpdateFile(filePath, fileContent, branchName, commitMsg);

    const pr = await createPullRequest(prTitle, prBody, branchName);

    console.log(`[appchain-registry] PR created: ${pr.prUrl} for ${metadata.name}`);

    return res.json({
      success: true,
      prUrl: pr.prUrl,
      prNumber: pr.prNumber,
      filePath,
    });
  } catch (e) {
    console.error("[appchain-registry] Error:", e);
    return res.status(500).json({
      success: false,
      error: e.message,
      code: "GITHUB_API_ERROR",
    });
  }
});

/**
 * GET /api/appchain-registry/status/:prNumber
 */
router.get("/status/:prNumber", async (req, res) => {
  try {
    const prNumber = parseInt(req.params.prNumber);
    if (isNaN(prNumber)) {
      return res.status(400).json({ error: "Invalid PR number" });
    }

    const pr = await getPullRequest(prNumber);
    return res.json({
      prNumber: pr.number,
      state: pr.state,
      merged: pr.merged,
      mergeable: pr.mergeable,
      title: pr.title,
      htmlUrl: pr.html_url,
    });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

module.exports = router;

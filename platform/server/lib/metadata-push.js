/**
 * Metadata Push — Push deployment metadata to GitHub metadata repository.
 *
 * Creates/updates a JSON file in the tokamak-rollup-metadata-repository
 * at the path: tokamak-appchain-data/{l1ChainId}/tokamak-appchain/{proposerAddress}.json
 *
 * Uses the GitHub Contents API (requires GITHUB_TOKEN with repo write access).
 */

const REPO_OWNER = process.env.METADATA_REPO_OWNER || "tokamak-network";
const REPO_NAME = process.env.METADATA_REPO_NAME || "tokamak-rollup-metadata-repository";
const REPO_BRANCH = process.env.METADATA_REPO_BRANCH || "main";
const GITHUB_TOKEN = process.env.GITHUB_TOKEN || null;

/**
 * Build metadata JSON from a deployment record (matching tokamak-appchain-metadata schema).
 */
function buildMetadataJSON(deployment) {
  const socialLinks = {};
  if (deployment.social_links) {
    try {
      const parsed = JSON.parse(deployment.social_links);
      if (parsed && typeof parsed === "object") {
        Object.assign(socialLinks, parsed);
      }
    } catch { /* ignore */ }
  }

  let screenshots = [];
  if (deployment.screenshots) {
    try {
      screenshots = JSON.parse(deployment.screenshots);
    } catch { /* ignore */ }
  }

  const l1ChainId = deployment.l1_chain_id || 1;
  const stackType = deployment.program_slug || "tokamak-appchain";
  const identityContract = (deployment.proposer_address || "").toLowerCase();

  return {
    l1ChainId,
    l2ChainId: deployment.chain_id || 0,
    stackType,
    identityContract,
    name: deployment.name,
    description: deployment.description || null,
    rollupType: "zk",
    status: deployment.status === "active" ? "active" : "inactive",
    rpcUrl: deployment.rpc_url || null,
    explorerUrl: deployment.explorer_url || null,
    bridgeUrl: deployment.bridge_url || null,
    dashboardUrl: deployment.dashboard_url || null,
    nativeToken: {
      type: deployment.native_token_type || "eth",
      symbol: deployment.native_token_symbol || "ETH",
      decimals: deployment.native_token_decimals ?? 18,
      l1Address: deployment.native_token_l1_address || null,
    },
    l1Contracts: {
      OnChainProposer: identityContract,
      CommonBridge: deployment.bridge_address || null,
    },
    operator: {
      name: deployment.owner_name || null,
      website: socialLinks.website || null,
      socialLinks: Object.keys(socialLinks).length > 0 ? socialLinks : null,
    },
    screenshots,
    hashtags: deployment.hashtags ? (() => {
      try { return JSON.parse(deployment.hashtags); } catch { return []; }
    })() : [],
    supportResources: {
      dashboardUrl: deployment.dashboard_url || null,
      website: socialLinks.website || null,
      twitter: socialLinks.twitter || null,
      discord: socialLinks.discord || null,
      telegram: socialLinks.telegram || null,
      github: socialLinks.github || null,
    },
    metadata: {
      signedBy: deployment.owner_wallet || null,
      signature: null, // Wallet signing is a follow-up feature
      updatedAt: new Date().toISOString(),
    },
  };
}

/**
 * Get the file path in the metadata repo for a given deployment.
 */
function getRepoFilePath(deployment) {
  const l1ChainId = deployment.l1_chain_id || 1;
  const stackType = deployment.program_slug || "tokamak-appchain";
  const proposerAddr = (deployment.proposer_address || "").toLowerCase();
  if (!proposerAddr || !/^0x[0-9a-f]{40}$/.test(proposerAddr)) return null;
  return `tokamak-appchain-data/${l1ChainId}/${stackType}/${proposerAddr}.json`;
}

/**
 * Check if a file exists in the repo and get its SHA (for updates).
 */
async function getFileSHA(filePath) {
  if (!GITHUB_TOKEN) throw new Error("GITHUB_TOKEN is required for metadata push");

  const url = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/contents/${filePath}?ref=${REPO_BRANCH}`;
  const res = await fetch(url, {
    headers: {
      Accept: "application/vnd.github.v3+json",
      Authorization: `Bearer ${GITHUB_TOKEN}`,
      "User-Agent": "tokamak-platform-push",
    },
    signal: AbortSignal.timeout(10000),
  });

  if (res.status === 404) return null;
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`GitHub Contents API ${res.status}: ${text}`);
  }

  const data = await res.json();
  return data.sha;
}

/**
 * Push metadata JSON to the GitHub repository.
 * Creates or updates the file at the computed path.
 */
async function pushMetadataToRepo(deployment) {
  if (!GITHUB_TOKEN) throw new Error("GITHUB_TOKEN is required for metadata push");

  const filePath = getRepoFilePath(deployment);
  if (!filePath) throw new Error("Cannot determine repo file path: proposer_address is required");

  const metadata = buildMetadataJSON(deployment);
  const content = Buffer.from(JSON.stringify(metadata, null, 2)).toString("base64");

  // Check if file already exists (get SHA for update)
  const existingSHA = await getFileSHA(filePath);

  const safeName = (deployment.name || "unknown").replace(/[^\w\s\-_.]/g, "").slice(0, 80);
  const body = {
    message: existingSHA
      ? `Update metadata for ${safeName}`
      : `Add metadata for ${safeName}`,
    content,
    branch: REPO_BRANCH,
  };

  if (existingSHA) {
    body.sha = existingSHA;
  }

  const url = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/contents/${filePath}`;
  let res = await fetch(url, {
    method: "PUT",
    headers: {
      Accept: "application/vnd.github.v3+json",
      Authorization: `Bearer ${GITHUB_TOKEN}`,
      "Content-Type": "application/json",
      "User-Agent": "tokamak-platform-push",
    },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(15000),
  });

  // Retry once on 409 conflict (stale SHA from concurrent push)
  if (res.status === 409) {
    const retrySHA = await getFileSHA(filePath);
    if (retrySHA) body.sha = retrySHA;
    res = await fetch(url, {
      method: "PUT",
      headers: {
        Accept: "application/vnd.github.v3+json",
        Authorization: `Bearer ${GITHUB_TOKEN}`,
        "Content-Type": "application/json",
        "User-Agent": "tokamak-platform-push",
      },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(15000),
    });
  }

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`GitHub Contents API PUT ${res.status}: ${text}`);
  }

  console.log(`[metadata-push] ${existingSHA ? "Updated" : "Created"} ${filePath}`);
  return { path: filePath, created: !existingSHA };
}

/**
 * Delete metadata file from the GitHub repository.
 */
async function deleteMetadataFromRepo(deployment) {
  if (!GITHUB_TOKEN) throw new Error("GITHUB_TOKEN is required for metadata delete");

  const filePath = getRepoFilePath(deployment);
  if (!filePath) throw new Error("Cannot determine repo file path");

  const sha = await getFileSHA(filePath);
  if (!sha) {
    console.log(`[metadata-push] File not found, nothing to delete: ${filePath}`);
    return { deleted: false };
  }

  const url = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/contents/${filePath}`;
  const res = await fetch(url, {
    method: "DELETE",
    headers: {
      Accept: "application/vnd.github.v3+json",
      Authorization: `Bearer ${GITHUB_TOKEN}`,
      "Content-Type": "application/json",
      "User-Agent": "tokamak-platform-push",
    },
    body: JSON.stringify({
      message: `Remove metadata for ${(deployment.name || "unknown").replace(/[^\w\s\-_.]/g, "").slice(0, 80)}`,
      sha,
      branch: REPO_BRANCH,
    }),
    signal: AbortSignal.timeout(15000),
  });

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`GitHub Contents API DELETE ${res.status}: ${text}`);
  }

  console.log(`[metadata-push] Deleted ${filePath}`);
  return { deleted: true };
}

module.exports = { pushMetadataToRepo, deleteMetadataFromRepo, buildMetadataJSON, getRepoFilePath };

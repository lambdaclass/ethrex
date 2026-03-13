const { getDb } = require("./db");

/**
 * Generate a deterministic listing ID from identity fields.
 */
function listingId(l1ChainId, stackType, identityContract) {
  return `${l1ChainId}-${stackType}-${identityContract.toLowerCase()}`;
}

/**
 * Upsert a listing from metadata-repo JSON.
 */
function upsertListing(metadata, repoFilePath) {
  const db = getDb();
  const id = listingId(metadata.l1ChainId, metadata.stackType, metadata.identityContract);
  const now = Date.now();

  db.prepare(`
    INSERT INTO explore_listings (
      id, l1_chain_id, l2_chain_id, stack_type, identity_contract,
      name, rollup_type, status, rpc_url, explorer_url, bridge_url, dashboard_url,
      native_token_type, native_token_symbol, native_token_decimals, native_token_l1_address,
      l1_contracts, operator_name, operator_website, operator_social_links,
      description, screenshots, hashtags,
      signed_by, signature, owner_wallet,
      repo_file_path, synced_at, created_at
    ) VALUES (
      ?, ?, ?, ?, ?,
      ?, ?, ?, ?, ?, ?, ?,
      ?, ?, ?, ?,
      ?, ?, ?, ?,
      ?, ?, ?,
      ?, ?, ?,
      ?, ?, ?
    )
    ON CONFLICT(id) DO UPDATE SET
      name = excluded.name,
      rollup_type = excluded.rollup_type,
      status = excluded.status,
      rpc_url = excluded.rpc_url,
      explorer_url = excluded.explorer_url,
      bridge_url = excluded.bridge_url,
      dashboard_url = excluded.dashboard_url,
      native_token_type = excluded.native_token_type,
      native_token_symbol = excluded.native_token_symbol,
      native_token_decimals = excluded.native_token_decimals,
      native_token_l1_address = excluded.native_token_l1_address,
      l1_contracts = excluded.l1_contracts,
      operator_name = excluded.operator_name,
      operator_website = excluded.operator_website,
      operator_social_links = excluded.operator_social_links,
      description = excluded.description,
      screenshots = excluded.screenshots,
      hashtags = excluded.hashtags,
      signed_by = excluded.signed_by,
      signature = excluded.signature,
      owner_wallet = excluded.owner_wallet,
      repo_file_path = excluded.repo_file_path,
      synced_at = excluded.synced_at
  `).run(
    id, metadata.l1ChainId, metadata.l2ChainId, metadata.stackType,
    metadata.identityContract?.toLowerCase(),
    metadata.name, metadata.rollupType || null, metadata.status || "active",
    metadata.rpcUrl || null, metadata.explorerUrl || null,
    metadata.bridgeUrl || null, metadata.dashboardUrl || null,
    metadata.nativeToken?.type || "eth",
    metadata.nativeToken?.symbol || "ETH",
    metadata.nativeToken?.decimals ?? 18,
    metadata.nativeToken?.l1Address || null,
    metadata.l1Contracts ? JSON.stringify(metadata.l1Contracts) : null,
    metadata.operator?.name || null,
    metadata.operator?.website || null,
    metadata.operator?.socialLinks ? JSON.stringify(metadata.operator.socialLinks) : null,
    metadata.description || null,
    metadata.screenshots ? JSON.stringify(metadata.screenshots) : null,
    metadata.hashtags ? JSON.stringify(metadata.hashtags) : null,
    metadata.metadata?.signedBy || null,
    metadata.metadata?.signature || null,
    metadata.metadata?.signedBy || null, // owner_wallet = signer
    repoFilePath || null,
    now, now,
  );

}

/**
 * Upsert listing and store the blob SHA for change detection.
 */
function upsertListingWithSha(metadata, repoFilePath, sha) {
  upsertListing(metadata, repoFilePath);
  if (sha) {
    const db = getDb();
    db.prepare("UPDATE explore_listings SET repo_sha = ? WHERE repo_file_path = ?").run(sha, repoFilePath);
  }
}

/**
 * Get a listing by ID.
 */
function getListingById(id) {
  const db = getDb();
  return db.prepare("SELECT * FROM explore_listings WHERE id = ?").get(id);
}

/**
 * Get all active listings with optional search and filters.
 */
function getListings({ limit = 50, offset = 0, search, stackType, l1ChainId } = {}) {
  const db = getDb();
  let sql = "SELECT * FROM explore_listings WHERE status = 'active'";
  const params = [];

  if (search) {
    sql += " AND (name LIKE ? OR operator_name LIKE ?)";
    params.push(`%${search}%`, `%${search}%`);
  }
  if (stackType) {
    sql += " AND stack_type = ?";
    params.push(stackType);
  }
  if (l1ChainId) {
    sql += " AND l1_chain_id = ?";
    params.push(parseInt(l1ChainId));
  }

  sql += " ORDER BY created_at DESC LIMIT ? OFFSET ?";
  params.push(limit, offset);
  return db.prepare(sql).all(...params);
}

/**
 * Get a listing by identity contract address and L1 chain ID (direct index lookup).
 */
function getListingByIdentityContract(identityContract, l1ChainId) {
  const db = getDb();
  return db.prepare(
    "SELECT * FROM explore_listings WHERE identity_contract = ? AND l1_chain_id = ? AND status = 'active'"
  ).get(identityContract.toLowerCase(), l1ChainId);
}

/**
 * Get all repo file paths + SHA for sync change detection.
 */
function getAllRepoFilePaths() {
  const db = getDb();
  return db.prepare("SELECT id, repo_file_path, repo_sha FROM explore_listings WHERE repo_file_path IS NOT NULL").all();
}

/**
 * Get proposer/identity addresses for a given L1 chain ID (for L1 indexer).
 */
function getListingAddressesForChain(l1ChainId) {
  const db = getDb();
  return db.prepare(
    "SELECT id, identity_contract FROM explore_listings WHERE l1_chain_id = ? AND status = 'active' AND identity_contract IS NOT NULL"
  ).all(l1ChainId).map((r) => r.identity_contract);
}

/**
 * Delete a listing by ID.
 */
function deleteListing(id) {
  const db = getDb();
  db.prepare("DELETE FROM explore_listings WHERE id = ?").run(id);
}

/**
 * Update listing visibility/status.
 */
function updateListingStatus(id, status) {
  const db = getDb();
  db.prepare("UPDATE explore_listings SET status = ? WHERE id = ?").run(status, id);
}

/**
 * Update listing with enrichment data (from L1 indexer / IPFS).
 */
function updateListingEnrichment(id, fields) {
  const db = getDb();
  const allowed = ["description", "screenshots", "explorer_url", "dashboard_url", "bridge_url"];
  const updates = [];
  const values = [];
  for (const [key, value] of Object.entries(fields)) {
    if (allowed.includes(key)) {
      updates.push(`${key} = ?`);
      values.push(typeof value === "object" ? JSON.stringify(value) : value);
    }
  }
  if (updates.length === 0) return;
  values.push(id);
  db.prepare(`UPDATE explore_listings SET ${updates.join(", ")} WHERE id = ?`).run(...values);
}

module.exports = {
  listingId,
  upsertListing,
  upsertListingWithSha,
  getListingById,
  getListingByIdentityContract,
  getListings,
  getAllRepoFilePaths,
  getListingAddressesForChain,
  deleteListing,
  updateListingStatus,
  updateListingEnrichment,
};

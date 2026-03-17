/**
 * Vercel Cron: Metadata sync from GitHub repository.
 * Polls tokamak-rollup-metadata-repository for appchain metadata changes.
 *
 * Supports two repo layouts:
 *   - data/{network}/{address}.json          (Thanos appchains, current)
 *   - tokamak-appchain-data/{chainId}/{stackType}/{address}.json (ethrex, future)
 */
import { NextRequest, NextResponse } from "next/server";
import { sql, sqlRaw, ensureSchema } from "@/lib/db";

const REPO_OWNER = process.env.METADATA_REPO_OWNER || "tokamak-network";
const REPO_NAME = process.env.METADATA_REPO_NAME || "tokamak-rollup-metadata-repository";
const REPO_BRANCH = process.env.METADATA_REPO_BRANCH || "main";
const GITHUB_TOKEN = process.env.GITHUB_TOKEN || null;

// Network name → L1 chain ID mapping
const NETWORK_CHAIN_IDS: Record<string, number> = {
  mainnet: 1,
  sepolia: 11155111,
  holesky: 17000,
};

function githubHeaders(): Record<string, string> {
  const headers: Record<string, string> = {
    Accept: "application/vnd.github.v3+json",
    "User-Agent": "tokamak-platform-sync",
  };
  if (GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${GITHUB_TOKEN}`;
  }
  return headers;
}

interface TreeItem {
  path: string;
  sha: string;
  url: string;
}

async function fetchRepoTree(): Promise<TreeItem[]> {
  const url = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/git/trees/${REPO_BRANCH}?recursive=1`;
  const res = await fetch(url, {
    headers: githubHeaders(),
    signal: AbortSignal.timeout(15000),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`GitHub Trees API ${res.status}: ${text}`);
  }
  const data = await res.json();
  return data.tree
    .filter((item: { type: string; path: string }) =>
      item.type === "blob" &&
      item.path.endsWith(".json") &&
      (item.path.startsWith("data/") || item.path.startsWith("tokamak-appchain-data/"))
    )
    .map((item: { path: string; sha: string; url: string }) => ({
      path: item.path,
      sha: item.sha,
      url: item.url,
    }));
}

async function fetchBlobContent(blobUrl: string) {
  const res = await fetch(blobUrl, {
    headers: githubHeaders(),
    signal: AbortSignal.timeout(10000),
  });
  if (!res.ok) throw new Error(`GitHub Blob API ${res.status}`);
  const blob = await res.json();
  const content = Buffer.from(blob.content, "base64").toString("utf-8");
  return JSON.parse(content);
}

/**
 * Parse file path from either layout:
 *   data/{network}/{address}.json
 *   tokamak-appchain-data/{chainId}/{stackType}/{address}.json
 */
function parseFilePath(filePath: string): { l1ChainId: number; stackType: string; identityContract: string } | null {
  // Layout 1: data/{network}/{address}.json (Thanos)
  const m1 = filePath.match(/^data\/([a-z]+)\/(0x[a-fA-F0-9]{40})\.json$/);
  if (m1) {
    const network = m1[1];
    const l1ChainId = NETWORK_CHAIN_IDS[network];
    if (!l1ChainId) return null;
    return { l1ChainId, stackType: "thanos", identityContract: m1[2] };
  }

  // Layout 2: tokamak-appchain-data/{chainId}/{stackType}/{address}.json (ethrex)
  const m2 = filePath.match(/^tokamak-appchain-data\/(\d+)\/([a-z0-9-]+)\/(0x[a-fA-F0-9]{40})\.json$/);
  if (m2) {
    return { l1ChainId: parseInt(m2[1]), stackType: m2[2], identityContract: m2[3] };
  }

  return null;
}

function listingId(l1ChainId: number, stackType: string, identityContract: string) {
  return `${l1ChainId}-${stackType}-${identityContract.toLowerCase()}`;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function normalizeMetadata(raw: any, pathInfo: { l1ChainId: number; stackType: string; identityContract: string }) {
  // Map actual repo JSON fields to our DB schema
  return {
    l1ChainId: raw.l1ChainId ?? pathInfo.l1ChainId,
    l2ChainId: raw.l2ChainId ?? 0,
    stackType: raw.stack?.name || raw.stackType || pathInfo.stackType,
    identityContract: pathInfo.identityContract,
    name: raw.name || "Unknown",
    rollupType: raw.rollupType || null,
    status: raw.status || "active",
    rpcUrl: raw.rpcUrl || null,
    explorerUrl: raw.explorers?.[0]?.url || raw.explorerUrl || null,
    bridgeUrl: raw.bridges?.[0]?.url || raw.bridgeUrl || null,
    dashboardUrl: raw.dashboardUrl || null,
    nativeTokenType: raw.nativeToken?.type || "eth",
    nativeTokenSymbol: raw.nativeToken?.symbol || "ETH",
    nativeTokenDecimals: raw.nativeToken?.decimals ?? 18,
    nativeTokenL1Address: raw.nativeToken?.l1Address || null,
    l1Contracts: raw.l1Contracts ? JSON.stringify(raw.l1Contracts) : null,
    operatorName: raw.operator?.name || null,
    operatorWebsite: raw.operator?.website || raw.website || null,
    operatorSocialLinks: raw.operator?.socialLinks
      ? JSON.stringify(raw.operator.socialLinks)
      : raw.supportResources
        ? JSON.stringify(raw.supportResources)
        : null,
    description: raw.description || null,
    screenshots: raw.screenshots ? JSON.stringify(raw.screenshots) : null,
    hashtags: raw.hashtags ? JSON.stringify(raw.hashtags) : null,
    signedBy: raw.metadata?.signedBy || null,
    signature: raw.metadata?.signature || null,
    ownerWallet: raw.metadata?.signedBy || null,
    createdAt: raw.createdAt || null,
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function upsertListing(metadata: any, repoFilePath: string, sha: string) {
  const id = listingId(metadata.l1ChainId, metadata.stackType, metadata.identityContract);
  const now = Date.now();

  await sqlRaw(
    `INSERT INTO explore_listings (
      id, l1_chain_id, l2_chain_id, stack_type, identity_contract,
      name, rollup_type, status, rpc_url, explorer_url, bridge_url, dashboard_url,
      native_token_type, native_token_symbol, native_token_decimals, native_token_l1_address,
      l1_contracts, operator_name, operator_website, operator_social_links,
      description, screenshots, hashtags,
      signed_by, signature, owner_wallet,
      repo_file_path, repo_sha, synced_at, created_at
    ) VALUES (
      $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30
    )
    ON CONFLICT(id) DO UPDATE SET
      name=EXCLUDED.name, rollup_type=EXCLUDED.rollup_type, status=EXCLUDED.status,
      rpc_url=EXCLUDED.rpc_url, explorer_url=EXCLUDED.explorer_url, bridge_url=EXCLUDED.bridge_url,
      dashboard_url=EXCLUDED.dashboard_url, native_token_type=EXCLUDED.native_token_type,
      native_token_symbol=EXCLUDED.native_token_symbol, native_token_decimals=EXCLUDED.native_token_decimals,
      native_token_l1_address=EXCLUDED.native_token_l1_address, l1_contracts=EXCLUDED.l1_contracts,
      operator_name=EXCLUDED.operator_name, operator_website=EXCLUDED.operator_website,
      operator_social_links=EXCLUDED.operator_social_links, description=EXCLUDED.description,
      screenshots=EXCLUDED.screenshots, hashtags=EXCLUDED.hashtags,
      signed_by=EXCLUDED.signed_by, signature=EXCLUDED.signature, owner_wallet=EXCLUDED.owner_wallet,
      repo_file_path=EXCLUDED.repo_file_path, repo_sha=EXCLUDED.repo_sha, synced_at=EXCLUDED.synced_at`,
    [
      id,
      metadata.l1ChainId,
      metadata.l2ChainId,
      metadata.stackType,
      metadata.identityContract?.toLowerCase(),
      metadata.name,
      metadata.rollupType,
      metadata.status,
      metadata.rpcUrl,
      metadata.explorerUrl,
      metadata.bridgeUrl,
      metadata.dashboardUrl,
      metadata.nativeTokenType,
      metadata.nativeTokenSymbol,
      metadata.nativeTokenDecimals,
      metadata.nativeTokenL1Address,
      metadata.l1Contracts,
      metadata.operatorName,
      metadata.operatorWebsite,
      metadata.operatorSocialLinks,
      metadata.description,
      metadata.screenshots,
      metadata.hashtags,
      metadata.signedBy,
      metadata.signature,
      metadata.ownerWallet,
      repoFilePath || null,
      sha || null,
      now,
      metadata.createdAt ? new Date(metadata.createdAt).getTime() : now,
    ]
  );
}

const COOLDOWN_MS = 5 * 60 * 1000; // 5 minutes

export async function GET(req: NextRequest) {
  // Verify cron secret (Vercel sets this header) — manual requests skip auth
  const authHeader = req.headers.get("authorization");
  const cronSecret = process.env.CRON_SECRET;
  const isCron = cronSecret && authHeader === `Bearer ${cronSecret}`;

  try {
    await ensureSchema();

    // Manual requests: enforce 5-minute cooldown
    if (!isCron) {
      const { rows } = await sql`SELECT value FROM platform_meta WHERE key = 'last_metadata_sync'`;
      if (rows.length > 0) {
        const lastSync = parseInt(rows[0].value as string, 10);
        const elapsed = Date.now() - lastSync;
        if (elapsed < COOLDOWN_MS) {
          const remainSec = Math.ceil((COOLDOWN_MS - elapsed) / 1000);
          return NextResponse.json(
            { error: `Please wait ${remainSec}s before syncing again`, cooldown: true, remain_sec: remainSec },
            { status: 429 }
          );
        }
      }
    }

    const startTime = Date.now();
    let synced = 0;
    let deleted = 0;
    let errors = 0;
    let firstError: string | null = null;

    // 1. Fetch current tree from GitHub
    const treeItems = await fetchRepoTree();
    const currentPaths = new Set(treeItems.map((item) => item.path));

    // 2. Get existing listings for deletion detection + SHA comparison
    const { rows: existingListings } = await sql`
      SELECT id, repo_file_path, repo_sha FROM explore_listings WHERE repo_file_path IS NOT NULL
    `;
    const existingShaMap = new Map(
      existingListings.map((l) => [l.repo_file_path as string, l.repo_sha as string])
    );

    // 3. Filter to only changed files
    const changedItems = treeItems.filter((item) => existingShaMap.get(item.path) !== item.sha);

    // 4. Fetch and upsert changed files with bounded concurrency
    const CONCURRENCY = 5;
    for (let i = 0; i < changedItems.length; i += CONCURRENCY) {
      const batch = changedItems.slice(i, i + CONCURRENCY);
      const results = await Promise.allSettled(
        batch.map(async (item) => {
          const pathInfo = parseFilePath(item.path);
          if (!pathInfo) return;

          const raw = await fetchBlobContent(item.url);
          const metadata = normalizeMetadata(raw, pathInfo);

          await upsertListing(metadata, item.path, item.sha);
          synced++;
        })
      );
      const errorMessages: string[] = [];
      for (const result of results) {
        if (result.status === "rejected") {
          const msg = result.reason?.message || String(result.reason);
          console.error(`[metadata-sync] Error:`, msg);
          errorMessages.push(msg);
          errors++;
        }
      }
      if (errorMessages.length > 0) {
        firstError = firstError || errorMessages[0];
      }
    }

    // 5. Detect and remove deleted files
    for (const listing of existingListings) {
      if (listing.repo_file_path && !currentPaths.has(listing.repo_file_path as string)) {
        await sql`DELETE FROM explore_listings WHERE id = ${listing.id}`;
        deleted++;
      }
    }

    // Record sync timestamp
    await sqlRaw(
      `INSERT INTO platform_meta (key, value) VALUES ('last_metadata_sync', $1)
       ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value`,
      [String(Date.now())]
    );

    const elapsed = Date.now() - startTime;
    return NextResponse.json({
      ok: true,
      synced,
      deleted,
      errors,
      total_files: treeItems.length,
      changed_files: changedItems.length,
      first_error: firstError,
      elapsed_ms: elapsed,
    });
  } catch (e) {
    console.error("[metadata-sync] Failed:", e);
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

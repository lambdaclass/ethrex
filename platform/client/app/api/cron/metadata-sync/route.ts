/**
 * Vercel Cron: Metadata sync from GitHub repository.
 * Polls tokamak-rollup-metadata-repository for appchain metadata changes.
 *
 * Schedule: every 5 minutes (Pro) or every hour (Hobby)
 */
import { NextRequest, NextResponse } from "next/server";
import { sql, sqlRaw, ensureSchema } from "@/lib/db";

const REPO_OWNER = process.env.METADATA_REPO_OWNER || "tokamak-network";
const REPO_NAME = process.env.METADATA_REPO_NAME || "tokamak-rollup-metadata-repository";
const REPO_BRANCH = process.env.METADATA_REPO_BRANCH || "main";
const DATA_PREFIX = "tokamak-appchain-data/";
const GITHUB_TOKEN = process.env.GITHUB_TOKEN || null;

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
      item.type === "blob" && item.path.startsWith(DATA_PREFIX) && item.path.endsWith(".json")
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

function parseFilePath(filePath: string) {
  const match = filePath.match(
    /^tokamak-appchain-data\/(\d+)\/([a-z0-9-]+)\/(0x[a-fA-F0-9]{40})\.json$/
  );
  if (!match) return null;
  return { l1ChainId: parseInt(match[1]), stackType: match[2], identityContract: match[3] };
}

function listingId(l1ChainId: number, stackType: string, identityContract: string) {
  return `${l1ChainId}-${stackType}-${identityContract.toLowerCase()}`;
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
      metadata.rollupType || null,
      metadata.status || "active",
      metadata.rpcUrl || null,
      metadata.explorerUrl || null,
      metadata.bridgeUrl || null,
      metadata.dashboardUrl || null,
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
      metadata.metadata?.signedBy || null,
      repoFilePath || null,
      sha || null,
      now,
      now,
    ]
  );
}

export async function GET(req: NextRequest) {
  // Verify cron secret (Vercel sets this header)
  const authHeader = req.headers.get("authorization");
  const cronSecret = process.env.CRON_SECRET;
  if (cronSecret && authHeader !== `Bearer ${cronSecret}`) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  try {
    await ensureSchema();

    const startTime = Date.now();
    let synced = 0;
    let deleted = 0;
    let errors = 0;

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

          const metadata = await fetchBlobContent(item.url);
          if (!metadata.identityContract) {
            metadata.identityContract = pathInfo.identityContract;
          }

          await upsertListing(metadata, item.path, item.sha);
          synced++;
        })
      );
      for (const result of results) {
        if (result.status === "rejected") {
          console.error(`[metadata-sync] Error:`, result.reason?.message);
          errors++;
        }
      }
    }

    // 5. Detect and remove deleted files
    for (const listing of existingListings) {
      if (listing.repo_file_path && !currentPaths.has(listing.repo_file_path as string)) {
        await sql`DELETE FROM explore_listings WHERE id = ${listing.id}`;
        deleted++;
      }
    }

    const elapsed = Date.now() - startTime;
    return NextResponse.json({
      ok: true,
      synced,
      deleted,
      errors,
      elapsed_ms: elapsed,
    });
  } catch (e) {
    console.error("[metadata-sync] Failed:", e);
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

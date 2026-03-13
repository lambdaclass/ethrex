/**
 * Metadata Sync — Polls tokamak-rollup-metadata-repository for appchain metadata.
 *
 * Uses the GitHub Git Trees API to efficiently list all files under
 * tokamak-appchain-data/, then fetches only changed files.
 *
 * Flow: GitHub API → parse JSON → upsert explore_listings → detect deletions
 */

const { upsertListing, getAllRepoFilePaths, deleteListing } = require("../db/listings");

const REPO_OWNER = process.env.METADATA_REPO_OWNER || "tokamak-network";
const REPO_NAME = process.env.METADATA_REPO_NAME || "tokamak-rollup-metadata-repository";
const REPO_BRANCH = process.env.METADATA_REPO_BRANCH || "main";
const DATA_PREFIX = "tokamak-appchain-data/";
const SYNC_INTERVAL = parseInt(process.env.METADATA_SYNC_INTERVAL) || 5 * 60 * 1000; // 5 minutes

// Optional GitHub token for higher rate limits (60/hr unauthenticated → 5000/hr)
const GITHUB_TOKEN = process.env.GITHUB_TOKEN || null;

function githubHeaders() {
  const headers = {
    Accept: "application/vnd.github.v3+json",
    "User-Agent": "tokamak-platform-sync",
  };
  if (GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${GITHUB_TOKEN}`;
  }
  return headers;
}

/**
 * Fetch the Git tree for the repo, filtered to tokamak-appchain-data/ JSON files.
 * Returns array of { path, sha, url } for each metadata JSON.
 */
async function fetchRepoTree() {
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
    .filter((item) => item.type === "blob" && item.path.startsWith(DATA_PREFIX) && item.path.endsWith(".json"))
    .map((item) => ({
      path: item.path,
      sha: item.sha,
      url: item.url, // blob URL for fetching content
    }));
}

/**
 * Fetch raw JSON content from a GitHub blob URL.
 */
async function fetchBlobContent(blobUrl) {
  const res = await fetch(blobUrl, {
    headers: githubHeaders(),
    signal: AbortSignal.timeout(10000),
  });

  if (!res.ok) {
    throw new Error(`GitHub Blob API ${res.status}`);
  }

  const blob = await res.json();
  // Blob content is base64-encoded
  const content = Buffer.from(blob.content, "base64").toString("utf-8");
  return JSON.parse(content);
}

/**
 * Parse file path to extract identity contract.
 * Path format: tokamak-appchain-data/{l1ChainId}/{stackType}/{identityContract}.json
 */
function parseFilePath(filePath) {
  const match = filePath.match(
    /^tokamak-appchain-data\/(\d+)\/([a-z0-9-]+)\/(0x[a-f0-9]{40})\.json$/
  );
  if (!match) return null;
  return {
    l1ChainId: parseInt(match[1]),
    stackType: match[2],
    identityContract: match[3],
  };
}

/**
 * Run a single sync cycle.
 */
async function syncOnce() {
  const startTime = Date.now();
  let synced = 0;
  let deleted = 0;
  let errors = 0;

  try {
    // 1. Fetch current tree from GitHub
    const treeItems = await fetchRepoTree();
    const currentPaths = new Set(treeItems.map((item) => item.path));

    // 2. Get existing listings for deletion detection
    const existingListings = getAllRepoFilePaths();

    // 3. Fetch and upsert each metadata file
    for (const item of treeItems) {
      try {
        const pathInfo = parseFilePath(item.path);
        if (!pathInfo) {
          console.warn(`[metadata-sync] Skipping invalid path: ${item.path}`);
          continue;
        }

        const metadata = await fetchBlobContent(item.url);

        // Inject identity contract from path if not in metadata
        if (!metadata.identityContract) {
          metadata.identityContract = pathInfo.identityContract;
        }

        upsertListing(metadata, item.path);
        synced++;
      } catch (err) {
        console.error(`[metadata-sync] Error processing ${item.path}:`, err.message);
        errors++;
      }
    }

    // 4. Detect and remove deleted files
    for (const listing of existingListings) {
      if (listing.repo_file_path && !currentPaths.has(listing.repo_file_path)) {
        deleteListing(listing.id);
        deleted++;
        console.log(`[metadata-sync] Deleted listing ${listing.id} (file removed from repo)`);
      }
    }

    const elapsed = Date.now() - startTime;
    console.log(
      `[metadata-sync] Sync complete: ${synced} synced, ${deleted} deleted, ${errors} errors (${elapsed}ms)`
    );
  } catch (err) {
    console.error(`[metadata-sync] Sync cycle failed:`, err.message);
  }
}

/**
 * Start the metadata sync polling loop.
 */
function startMetadataSync(intervalMs = SYNC_INTERVAL) {
  console.log(
    `[metadata-sync] Starting sync from ${REPO_OWNER}/${REPO_NAME}@${REPO_BRANCH} (interval: ${intervalMs / 1000}s)`
  );

  // Initial sync after a short delay (let server finish starting)
  const initialDelay = setTimeout(() => {
    syncOnce();
  }, 5000);

  // Polling loop
  let syncing = false;
  const interval = setInterval(async () => {
    if (syncing) return;
    syncing = true;
    try {
      await syncOnce();
    } finally {
      syncing = false;
    }
  }, intervalMs);

  return { interval, initialDelay };
}

module.exports = { startMetadataSync, syncOnce };

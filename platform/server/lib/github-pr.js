/**
 * GitHub PR Helper — Create branches, commit files, and open PRs
 * via the GitHub REST API.
 */

const REPO_OWNER = process.env.METADATA_REPO_OWNER || "tokamak-network";
const REPO_NAME =
  process.env.METADATA_REPO_NAME || "tokamak-rollup-metadata-repository";
const GITHUB_TOKEN = process.env.GITHUB_BOT_TOKEN || process.env.GITHUB_TOKEN || null;

const HEADERS = {
  Accept: "application/vnd.github.v3+json",
  "Content-Type": "application/json",
  "User-Agent": "tokamak-platform-registry",
};

function authHeaders() {
  if (!GITHUB_TOKEN) throw new Error("GITHUB_BOT_TOKEN is required");
  return { ...HEADERS, Authorization: `Bearer ${GITHUB_TOKEN}` };
}

const API = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}`;

/**
 * Get the SHA of a branch (usually main).
 */
async function getBranchSHA(branch = "main") {
  const res = await fetch(`${API}/git/ref/heads/${branch}`, {
    headers: authHeaders(),
    signal: AbortSignal.timeout(10000),
  });
  if (!res.ok) throw new Error(`getBranchSHA: ${res.status} ${await res.text()}`);
  const data = await res.json();
  return data.object.sha;
}

/**
 * Create a new branch from a base SHA.
 */
async function createBranch(branchName, baseSha) {
  const res = await fetch(`${API}/git/refs`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ ref: `refs/heads/${branchName}`, sha: baseSha }),
    signal: AbortSignal.timeout(10000),
  });
  if (res.status === 422) {
    // Branch already exists — ok
    return;
  }
  if (!res.ok) throw new Error(`createBranch: ${res.status} ${await res.text()}`);
}

/**
 * Create or update a file on a branch.
 * Returns the commit SHA.
 */
async function createOrUpdateFile(filePath, content, branch, commitMessage) {
  // Check if file exists on branch (for update SHA)
  let existingSHA = null;
  const checkRes = await fetch(
    `${API}/contents/${filePath}?ref=${branch}`,
    { headers: authHeaders(), signal: AbortSignal.timeout(10000) }
  );
  if (checkRes.ok) {
    const existing = await checkRes.json();
    existingSHA = existing.sha;
  }

  const body = {
    message: commitMessage,
    content: Buffer.from(content).toString("base64"),
    branch,
  };
  if (existingSHA) body.sha = existingSHA;

  const res = await fetch(`${API}/contents/${filePath}`, {
    method: "PUT",
    headers: authHeaders(),
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(15000),
  });
  if (!res.ok) throw new Error(`createOrUpdateFile: ${res.status} ${await res.text()}`);
  const data = await res.json();
  return data.commit.sha;
}

/**
 * Create a pull request.
 * Returns { prUrl, prNumber }.
 */
async function createPullRequest(title, body, head, base = "main") {
  const res = await fetch(`${API}/pulls`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ title, body, head, base }),
    signal: AbortSignal.timeout(15000),
  });
  if (!res.ok) throw new Error(`createPullRequest: ${res.status} ${await res.text()}`);
  const data = await res.json();

  // Enable auto-merge (squash) so GitHub merges after status checks pass
  try {
    // GitHub GraphQL API is needed for enablePullRequestAutoMerge
    const graphqlRes = await fetch("https://api.github.com/graphql", {
      method: "POST",
      headers: authHeaders(),
      body: JSON.stringify({
        query: `mutation {
          enablePullRequestAutoMerge(input: { pullRequestId: "${data.node_id}", mergeMethod: SQUASH }) {
            pullRequest { autoMergeRequest { enabledAt } }
          }
        }`,
      }),
      signal: AbortSignal.timeout(10000),
    });
    if (graphqlRes.ok) {
      console.log(`[github-pr] Auto-merge enabled for PR #${data.number}`);
    }
  } catch (e) {
    console.warn(`[github-pr] Failed to enable auto-merge: ${e.message}`);
  }

  return { prUrl: data.html_url, prNumber: data.number };
}

/**
 * Get PR info by number.
 */
async function getPullRequest(prNumber) {
  const res = await fetch(`${API}/pulls/${prNumber}`, {
    headers: authHeaders(),
    signal: AbortSignal.timeout(10000),
  });
  if (!res.ok) throw new Error(`getPullRequest: ${res.status} ${await res.text()}`);
  return res.json();
}

/**
 * Check if there's an open PR for a given file path (to prevent duplicates).
 */
async function findOpenPR(filePath) {
  // Search open PRs and verify by checking actual changed files
  const res = await fetch(
    `${API}/pulls?state=open&per_page=50`,
    { headers: authHeaders(), signal: AbortSignal.timeout(10000) }
  );
  if (!res.ok) return null;
  const prs = await res.json();
  const filename = filePath.split("/").pop().replace(".json", "");
  for (const pr of prs) {
    // Quick title check first to narrow candidates
    if (!pr.title.includes(filename)) continue;
    // Verify by checking actual files changed in the PR
    try {
      const filesRes = await fetch(
        `${API}/pulls/${pr.number}/files`,
        { headers: authHeaders(), signal: AbortSignal.timeout(10000) }
      );
      if (!filesRes.ok) continue;
      const files = await filesRes.json();
      if (files.some((f) => f.filename === filePath)) {
        return { prUrl: pr.html_url, prNumber: pr.number, headBranch: pr.head.ref };
      }
    } catch { /* skip */ }
  }
  return null;
}

/**
 * Merge a PR using squash merge.
 */
async function mergePullRequest(prNumber, commitTitle) {
  const res = await fetch(`${API}/pulls/${prNumber}/merge`, {
    method: "PUT",
    headers: authHeaders(),
    body: JSON.stringify({
      commit_title: commitTitle || `Auto-merge PR #${prNumber}`,
      merge_method: "squash",
    }),
    signal: AbortSignal.timeout(15000),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`mergePullRequest: ${res.status} ${text}`);
  }
  return res.json();
}

/**
 * Update a PR's title and/or body.
 */
async function updatePullRequest(prNumber, { title, body } = {}) {
  const updates = {};
  if (title) updates.title = title;
  if (body) updates.body = body;
  if (Object.keys(updates).length === 0) return;

  const res = await fetch(`${API}/pulls/${prNumber}`, {
    method: "PATCH",
    headers: authHeaders(),
    body: JSON.stringify(updates),
    signal: AbortSignal.timeout(10000),
  });
  if (!res.ok) throw new Error(`updatePullRequest: ${res.status} ${await res.text()}`);
  return res.json();
}

/**
 * Get file content from a branch (usually main).
 * Returns parsed JSON content or null if not found.
 */
async function getFileContent(filePath, branch = "main") {
  const res = await fetch(
    `${API}/contents/${filePath}?ref=${branch}`,
    { headers: authHeaders(), signal: AbortSignal.timeout(10000) }
  );
  if (!res.ok) return null;
  const data = await res.json();
  if (!data.content) return null;
  const decoded = Buffer.from(data.content, "base64").toString("utf-8");
  try {
    return JSON.parse(decoded);
  } catch {
    return null;
  }
}

module.exports = {
  getBranchSHA,
  createBranch,
  createOrUpdateFile,
  createPullRequest,
  getPullRequest,
  findOpenPR,
  mergePullRequest,
  updatePullRequest,
  getFileContent,
};

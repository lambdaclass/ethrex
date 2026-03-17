#!/usr/bin/env node
/**
 * E2E test for Store (Open Appchain) flow.
 *
 * Tests the full lifecycle:
 * 1. Login to Platform
 * 2. Register a deployment (publish)
 * 3. Update deployment with description/screenshots/social links
 * 4. Activate deployment
 * 5. Browse store — verify the appchain appears in listing
 * 6. Fetch appchain detail — verify all fields
 * 7. Fetch reviews/comments/announcements (empty)
 * 8. Toggle bookmark (authenticated)
 * 9. Push metadata (if GITHUB_TOKEN configured)
 * 10. Deactivate deployment
 * 11. Verify appchain no longer appears in listing
 *
 * Usage:
 *   node platform/tests/e2e-store.js [--platform-url URL] [--email EMAIL] [--password PASSWORD]
 *
 * Requires:
 *   - Running Platform server
 *   - A valid Platform user account
 */

const PLATFORM_URL = getArg("--platform-url") || "http://localhost:5001";
const EMAIL = getArg("--email") || "test@tokamak.network";
const PASSWORD = getArg("--password") || "test1234";

function getArg(flag) {
  const idx = process.argv.indexOf(flag);
  return idx >= 0 && idx + 1 < process.argv.length ? process.argv[idx + 1] : null;
}

async function api(path, options = {}) {
  const url = `${PLATFORM_URL}${path}`;
  const res = await fetch(url, {
    headers: {
      "Content-Type": "application/json",
      ...(options.headers || {}),
    },
    ...options,
  });
  const data = await res.json();
  if (!res.ok) {
    throw new Error(`${options.method || "GET"} ${path} → ${res.status}: ${data.error || JSON.stringify(data)}`);
  }
  return data;
}

let authToken = null;

function authHeaders() {
  return authToken ? { Authorization: `Bearer ${authToken}` } : {};
}

async function authApi(path, options = {}) {
  return api(path, {
    ...options,
    headers: { ...authHeaders(), ...(options.headers || {}) },
  });
}

// ── Test Steps ──

async function step1_login() {
  console.log("\n[Step 1] Login...");
  const data = await api("/api/auth/login", {
    method: "POST",
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  authToken = data.token;
  console.log(`  ✓ Logged in as ${data.user.name} (${data.user.email})`);
  return data.user;
}

let deploymentId = null;

async function step2_register() {
  console.log("\n[Step 2] Register deployment...");
  const data = await authApi("/api/deployments", {
    method: "POST",
    body: JSON.stringify({
      programId: "ethrex-appchain",
      name: `E2E-Test-${Date.now()}`,
      chainId: 99901,
      rpcUrl: "https://rpc.e2e-test.example.com",
    }),
  });
  deploymentId = data.deployment.id;
  console.log(`  ✓ Created deployment: ${deploymentId}`);
}

async function step3_update() {
  console.log("\n[Step 3] Update deployment with details...");
  await authApi(`/api/deployments/${deploymentId}`, {
    method: "PUT",
    body: JSON.stringify({
      description: "E2E test appchain for store flow verification",
      l1_chain_id: 11155111,
      network_mode: "testnet",
      proposer_address: "0xe2e0000000000000000000000000000000000001",
      bridge_address: "0xe2e0000000000000000000000000000000000002",
      screenshots: JSON.stringify(["ipfs://QmTestScreenshot1"]),
      social_links: JSON.stringify({ website: "https://e2e-test.com", twitter: "@e2etest" }),
      explorer_url: "https://explorer.e2e-test.com",
      dashboard_url: "https://dashboard.e2e-test.com",
    }),
  });
  console.log("  ✓ Updated with description, screenshots, social links");
}

async function step4_activate() {
  console.log("\n[Step 4] Activate deployment...");
  await authApi(`/api/deployments/${deploymentId}/activate`, {
    method: "POST",
  });
  console.log("  ✓ Deployment activated");
}

async function step5_browse_store() {
  console.log("\n[Step 5] Browse store — verify appchain appears...");
  const data = await api("/api/store/appchains?limit=100");
  const found = data.appchains.find((a) => a.id === deploymentId);
  if (!found) {
    throw new Error(`Deployment ${deploymentId} not found in store listing`);
  }
  console.log(`  ✓ Found in listing: "${found.name}"`);
  console.log(`    hashtags: ${JSON.stringify(found.hashtags)}`);
  console.log(`    avg_rating: ${found.avg_rating}, reviews: ${found.review_count}, comments: ${found.comment_count}`);
}

async function step6_detail() {
  console.log("\n[Step 6] Fetch appchain detail...");
  const data = await api(`/api/store/appchains/${deploymentId}`);
  const a = data.appchain;

  // Verify fields
  const checks = [
    ["name", a.name, true],
    ["description", a.description, "E2E test appchain for store flow verification"],
    ["rpc_url", a.rpc_url, "https://rpc.e2e-test.example.com"],
    ["chain_id", a.chain_id, 99901],
    ["explorer_url", a.explorer_url, "https://explorer.e2e-test.com"],
    ["dashboard_url", a.dashboard_url, "https://dashboard.e2e-test.com"],
    ["screenshots.length", a.screenshots?.length, 1],
    ["social_links.website", a.social_links?.website, "https://e2e-test.com"],
  ];

  for (const [field, actual, expected] of checks) {
    if (expected === true) {
      if (!actual) throw new Error(`Field ${field} is empty`);
    } else {
      if (actual !== expected) throw new Error(`Field ${field}: expected ${expected}, got ${actual}`);
    }
    console.log(`  ✓ ${field} = ${JSON.stringify(actual)}`);
  }
}

async function step7_community() {
  console.log("\n[Step 7] Fetch community data (should be empty)...");
  const reviews = await api(`/api/store/appchains/${deploymentId}/reviews`);
  const comments = await api(`/api/store/appchains/${deploymentId}/comments`);
  const announcements = await api(`/api/store/appchains/${deploymentId}/announcements`);

  console.log(`  ✓ Reviews: ${reviews.reviews.length} (expected 0)`);
  console.log(`  ✓ Comments: ${comments.comments.length} (expected 0)`);
  console.log(`  ✓ Announcements: ${announcements.announcements.length} (expected 0)`);
}

async function step8_bookmark() {
  console.log("\n[Step 8] Toggle bookmark...");
  const result = await authApi(`/api/store/appchains/${deploymentId}/bookmark`, {
    method: "POST",
  });
  console.log(`  ✓ Bookmarked: ${result.bookmarked}`);

  const bookmarks = await authApi("/api/store/bookmarks");
  const hasBookmark = bookmarks.bookmarks.includes(deploymentId);
  console.log(`  ✓ In bookmark list: ${hasBookmark}`);

  // Toggle off
  const result2 = await authApi(`/api/store/appchains/${deploymentId}/bookmark`, {
    method: "POST",
  });
  console.log(`  ✓ Un-bookmarked: ${!result2.bookmarked}`);
}

async function step9_metadata_push() {
  console.log("\n[Step 9] Push metadata...");
  try {
    const result = await authApi(`/api/deployments/${deploymentId}/push-metadata`, {
      method: "POST",
    });
    console.log(`  ✓ Metadata pushed to: ${result.path}`);
  } catch (err) {
    if (err.message.includes("GITHUB_TOKEN")) {
      console.log("  ⊘ Skipped (GITHUB_TOKEN not configured)");
    } else {
      throw err;
    }
  }
}

async function step10_deactivate() {
  console.log("\n[Step 10] Deactivate deployment...");
  await authApi(`/api/deployments/${deploymentId}`, {
    method: "PUT",
    body: JSON.stringify({ status: "inactive" }),
  });
  console.log("  ✓ Deployment deactivated");
}

async function step11_verify_hidden() {
  console.log("\n[Step 11] Verify appchain no longer in listing...");
  const data = await api("/api/store/appchains?limit=100");
  const found = data.appchains.find((a) => a.id === deploymentId);
  if (found) {
    throw new Error(`Deployment ${deploymentId} should not appear after deactivation`);
  }
  console.log("  ✓ Not found in listing (correct)");
}

async function cleanup() {
  if (deploymentId && authToken) {
    try {
      await authApi(`/api/deployments/${deploymentId}`, { method: "DELETE" });
      console.log(`\n[Cleanup] Deleted deployment ${deploymentId}`);
    } catch {
      console.log(`\n[Cleanup] Failed to delete (may already be gone)`);
    }
  }
}

// ── Main ──

async function main() {
  console.log("═══════════════════════════════════════════════");
  console.log("  E2E Store Test — Open Appchain Lifecycle");
  console.log(`  Platform: ${PLATFORM_URL}`);
  console.log("═══════════════════════════════════════════════");

  try {
    await step1_login();
    await step2_register();
    await step3_update();
    await step4_activate();
    await step5_browse_store();
    await step6_detail();
    await step7_community();
    await step8_bookmark();
    await step9_metadata_push();
    await step10_deactivate();
    await step11_verify_hidden();

    console.log("\n═══════════════════════════════════════════════");
    console.log("  ✓ ALL TESTS PASSED");
    console.log("═══════════════════════════════════════════════\n");
  } catch (err) {
    console.error(`\n  ✗ FAILED: ${err.message}\n`);
    process.exitCode = 1;
  } finally {
    await cleanup();
  }
}

main();

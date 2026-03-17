#!/usr/bin/env node
/**
 * Showroom Social Features — E2E Test Script
 *
 * Usage:
 *   node test-social.js          # seed + test (server must be running on :5001)
 *   node test-social.js seed     # seed dummy data only (no server needed)
 *   node test-social.js clean    # remove test data
 *
 * Prerequisites:
 *   cd platform/server && npm run dev   (terminal 1)
 *   cd platform/client && npm run dev   (terminal 2, optional — for browser testing)
 */

const API = process.env.API_URL || "http://localhost:5001";

// ── Seed: insert dummy deployment directly into DB ──

async function seed() {
  const { getDb } = require("./db/db");
  const db = getDb();

  // Use existing user and program
  const user = db.prepare("SELECT id FROM users WHERE name != 'System' LIMIT 1").get();
  const program = db.prepare("SELECT id FROM programs WHERE program_id = 'zk-dex' LIMIT 1").get();
  if (!user || !program) {
    console.error("ERROR: No users or programs in DB. Run the server once first.");
    process.exit(1);
  }

  const { randomUUID: uuid } = require("crypto");
  const deployments = [
    {
      id: "test-chain-alpha",
      name: "Alpha ZK Chain",
      chain_id: 49001,
      description: "A test ZK-DEX appchain for demonstrating social features.\nBuilt with Tokamak Appchain stack.",
      status: "active",
      network_mode: "testnet",
      l1_chain_id: 11155111,
      rpc_url: "https://rpc.example.com",
      bridge_address: "0x1234567890abcdef1234567890abcdef12345678",
      proposer_address: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
      social_links: JSON.stringify({ twitter: "https://twitter.com/tokamak", discord: "https://discord.gg/tokamak" }),
    },
    {
      id: "test-chain-beta",
      name: "Beta DeFi Chain",
      chain_id: 49002,
      description: "Second test appchain — a DeFi-focused L2.",
      status: "active",
      network_mode: "local",
      l1_chain_id: null,
      rpc_url: "http://localhost:1729",
      bridge_address: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
      proposer_address: "0xcafebabecafebabecafebabecafebabecafebabe",
      social_links: JSON.stringify({}),
    },
  ];

  const stmt = db.prepare(`
    INSERT OR REPLACE INTO deployments
      (id, user_id, program_id, name, chain_id, rpc_url, status, phase,
       description, bridge_address, proposer_address, social_links,
       l1_chain_id, network_mode, created_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, 'live', ?, ?, ?, ?, ?, ?, ?)
  `);

  for (const d of deployments) {
    stmt.run(
      d.id, user.id, program.id, d.name, d.chain_id, d.rpc_url, d.status,
      d.description, d.bridge_address, d.proposer_address, d.social_links,
      d.l1_chain_id, d.network_mode, Date.now()
    );
    console.log(`  Seeded: ${d.name} (${d.id})`);
  }

  console.log("\nDone. Visit http://localhost:3000/showroom to see them.\n");
}

// ── Clean: remove test data ──

function clean() {
  const { getDb } = require("./db/db");
  const db = getDb();
  const ids = ["test-chain-alpha", "test-chain-beta"];
  for (const id of ids) {
    db.prepare("DELETE FROM reactions WHERE target_id IN (SELECT id FROM reviews WHERE deployment_id = ?)").run(id);
    db.prepare("DELETE FROM reactions WHERE target_id IN (SELECT id FROM comments WHERE deployment_id = ?)").run(id);
    db.prepare("DELETE FROM reviews WHERE deployment_id = ?").run(id);
    db.prepare("DELETE FROM comments WHERE deployment_id = ?").run(id);
    db.prepare("DELETE FROM deployments WHERE id = ?").run(id);
  }
  console.log("Test data cleaned.");
}

// ── API Tests (requires server running) ──

let passed = 0;
let failed = 0;

function assert(condition, msg) {
  if (condition) {
    passed++;
    console.log(`  PASS: ${msg}`);
  } else {
    failed++;
    console.log(`  FAIL: ${msg}`);
  }
}

async function api(path, options = {}) {
  const res = await fetch(`${API}${path}`, {
    ...options,
    headers: { "Content-Type": "application/json", ...options.headers },
  });
  const data = await res.json();
  return { status: res.status, data };
}

async function runTests() {
  const { Wallet } = require("ethers");
  const { CHALLENGE_MESSAGE } = require("./lib/wallet-auth");

  // Create two test wallets
  const wallet1 = Wallet.createRandom();
  const wallet2 = Wallet.createRandom();
  const sig1 = wallet1.signMessageSync(CHALLENGE_MESSAGE);
  const sig2 = wallet2.signMessageSync(CHALLENGE_MESSAGE);

  const walletHeaders1 = {
    "x-wallet-address": wallet1.address,
    "x-wallet-signature": sig1,
  };
  const walletHeaders2 = {
    "x-wallet-address": wallet2.address,
    "x-wallet-signature": sig2,
  };

  const CHAIN_ID = "test-chain-alpha";

  console.log("\n── 1. Showroom List ──");
  {
    const { status, data } = await api("/api/store/appchains");
    assert(status === 200, "GET /appchains returns 200");
    assert(data.appchains.length >= 2, `Found ${data.appchains.length} appchains`);
    const alpha = data.appchains.find((a) => a.id === CHAIN_ID);
    assert(alpha !== undefined, "Alpha chain visible in list");
    assert(alpha.avg_rating === null, "No ratings yet → avg_rating is null");
    assert(alpha.review_count === 0, "review_count is 0");
    assert(alpha.comment_count === 0, "comment_count is 0");
  }

  console.log("\n── 2. Appchain Detail ──");
  {
    const { status, data } = await api(`/api/store/appchains/${CHAIN_ID}`);
    assert(status === 200, "GET /appchains/:id returns 200");
    assert(data.appchain.name === "Alpha ZK Chain", "Name correct");
    assert(data.appchain.social_links.twitter !== undefined, "Social links parsed");
  }

  console.log("\n── 3. Reviews ──");
  {
    // Create review (wallet1)
    const { status, data } = await api(`/api/store/appchains/${CHAIN_ID}/reviews`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ rating: 5, content: "Excellent chain! Love the ZK proofs." }),
    });
    assert(status === 201, "POST review returns 201");
    assert(data.review.rating === 5, "Rating saved correctly");
    assert(data.review.wallet_address === wallet1.address.toLowerCase(), "Wallet address lowercase");

    // Create another review (wallet2)
    const r2 = await api(`/api/store/appchains/${CHAIN_ID}/reviews`, {
      method: "POST",
      headers: walletHeaders2,
      body: JSON.stringify({ rating: 3, content: "Good but needs better docs." }),
    });
    assert(r2.status === 201, "Second review created");

    // Update review (wallet1, same wallet = upsert)
    const r3 = await api(`/api/store/appchains/${CHAIN_ID}/reviews`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ rating: 4, content: "Updated: still great after more testing." }),
    });
    assert(r3.status === 201, "Review upsert works");
    assert(r3.data.review.rating === 4, "Rating updated to 4");

    // List reviews
    const list = await api(`/api/store/appchains/${CHAIN_ID}/reviews`);
    assert(list.data.reviews.length === 2, "Two reviews total (not three — upsert)");

    // Validation: bad rating
    const bad = await api(`/api/store/appchains/${CHAIN_ID}/reviews`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ rating: 6, content: "test" }),
    });
    assert(bad.status === 400, "Rating > 5 rejected");

    // No auth
    const noAuth = await api(`/api/store/appchains/${CHAIN_ID}/reviews`, {
      method: "POST",
      body: JSON.stringify({ rating: 3, content: "test" }),
    });
    assert(noAuth.status === 401, "No wallet = 401");
  }

  console.log("\n── 4. Comments ──");
  {
    const { status, data } = await api(`/api/store/appchains/${CHAIN_ID}/comments`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ content: "Great work on this chain!" }),
    });
    assert(status === 201, "POST comment returns 201");

    const c2 = await api(`/api/store/appchains/${CHAIN_ID}/comments`, {
      method: "POST",
      headers: walletHeaders2,
      body: JSON.stringify({ content: "Thanks! Happy to help." }),
    });
    assert(c2.status === 201, "Second comment created");

    const list = await api(`/api/store/appchains/${CHAIN_ID}/comments`);
    assert(list.data.comments.length === 2, "Two comments");
    assert(list.data.comments[0].created_at < list.data.comments[1].created_at, "Comments ordered ASC");
  }

  console.log("\n── 5. Reactions (Likes) ──");
  {
    // Get reviews to find IDs
    const reviews = await api(`/api/store/appchains/${CHAIN_ID}/reviews`);
    const reviewId = reviews.data.reviews[0].id;

    // Like (toggle on)
    const like = await api(`/api/store/appchains/${CHAIN_ID}/reactions`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ targetType: "review", targetId: reviewId }),
    });
    assert(like.data.liked === true, "Like toggled ON");
    assert(like.data.count === 1, "Count is 1");

    // Second user likes too
    const like2 = await api(`/api/store/appchains/${CHAIN_ID}/reactions`, {
      method: "POST",
      headers: walletHeaders2,
      body: JSON.stringify({ targetType: "review", targetId: reviewId }),
    });
    assert(like2.data.count === 2, "Count is 2 after second like");

    // Toggle off (wallet1 unlikes)
    const unlike = await api(`/api/store/appchains/${CHAIN_ID}/reactions`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ targetType: "review", targetId: reviewId }),
    });
    assert(unlike.data.liked === false, "Like toggled OFF");
    assert(unlike.data.count === 1, "Count back to 1");

    // Reaction on nonexistent target
    const bad = await api(`/api/store/appchains/${CHAIN_ID}/reactions`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ targetType: "review", targetId: "nonexistent-id" }),
    });
    assert(bad.status === 404, "Reaction on nonexistent target = 404");
  }

  console.log("\n── 6. User Reactions in List ──");
  {
    const reviews = await api(`/api/store/appchains/${CHAIN_ID}/reviews`, {
      headers: { "x-wallet-address": wallet2.address.toLowerCase() },
    });
    assert(reviews.data.userReactions.length > 0, "User reactions returned for wallet2");
  }

  console.log("\n── 7. Social Stats in Appchain List ──");
  {
    const { data } = await api("/api/store/appchains");
    const alpha = data.appchains.find((a) => a.id === CHAIN_ID);
    assert(alpha.avg_rating !== null, `avg_rating = ${alpha.avg_rating}`);
    assert(alpha.review_count === 2, `review_count = ${alpha.review_count}`);
    assert(alpha.comment_count === 2, `comment_count = ${alpha.comment_count}`);
  }

  console.log("\n── 8. Delete ──");
  {
    const reviews = await api(`/api/store/appchains/${CHAIN_ID}/reviews`);
    const myReview = reviews.data.reviews.find(
      (r) => r.wallet_address === wallet1.address.toLowerCase()
    );

    // Can't delete someone else's
    const notMine = await api(`/api/store/appchains/${CHAIN_ID}/reviews/${myReview.id}`, {
      method: "DELETE",
      headers: walletHeaders2,
    });
    assert(notMine.status === 404, "Can't delete someone else's review");

    // Delete own
    const del = await api(`/api/store/appchains/${CHAIN_ID}/reviews/${myReview.id}`, {
      method: "DELETE",
      headers: walletHeaders1,
    });
    assert(del.status === 200, "Deleted own review");

    const after = await api(`/api/store/appchains/${CHAIN_ID}/reviews`);
    assert(after.data.reviews.length === 1, "One review remaining");
  }

  console.log("\n── 9. Edge Cases ──");
  {
    // Nonexistent appchain
    const { status } = await api("/api/store/appchains/nonexistent/reviews");
    assert(status === 404, "Nonexistent appchain = 404");

    // Empty content
    const empty = await api(`/api/store/appchains/${CHAIN_ID}/comments`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ content: "   " }),
    });
    assert(empty.status === 400, "Whitespace-only content rejected");

    // Content too long
    const long = await api(`/api/store/appchains/${CHAIN_ID}/comments`, {
      method: "POST",
      headers: walletHeaders1,
      body: JSON.stringify({ content: "x".repeat(501) }),
    });
    assert(long.status === 400, "Content > 500 chars rejected");
  }

  // Summary
  console.log(`\n${"=".repeat(40)}`);
  console.log(`Results: ${passed} passed, ${failed} failed`);
  console.log(`${"=".repeat(40)}\n`);

  if (failed > 0) process.exit(1);
}

// ── Main ──

async function main() {
  const cmd = process.argv[2];

  if (cmd === "seed") {
    console.log("Seeding test data...");
    seed();
  } else if (cmd === "clean") {
    clean();
  } else {
    // Default: seed + test
    console.log("Seeding test data...");
    seed();

    console.log("Running API tests against", API);
    try {
      await runTests();
    } finally {
      // Don't auto-clean — leave data for manual browser testing
      console.log("Test data left in DB for browser testing.");
      console.log("Run 'node test-social.js clean' to remove.\n");
    }
  }
}

main().catch((e) => {
  console.error("FATAL:", e.message);
  process.exit(1);
});

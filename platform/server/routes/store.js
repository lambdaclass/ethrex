const express = require("express");
const router = express.Router();

const { getActivePrograms, getProgramById, getCategories } = require("../db/programs");
const { getActiveDeployments, getActiveDeploymentById } = require("../db/deployments");
const { getListings, getListingById } = require("../db/listings");
const { requireWallet } = require("../lib/wallet-auth");
const {
  getReviewsByDeployment, createReview, deleteReview,
  getCommentsByDeployment, createComment, deleteComment,
  toggleReaction, getReactionCounts, getUserReactions,
  getSocialStats, getSocialStatsBatch, targetExists,
} = require("../db/social");
const { toggleBookmark, getUserBookmarks } = require("../db/bookmarks");
const { getAnnouncements, createAnnouncement, updateAnnouncement, deleteAnnouncement, getAnnouncementCount } = require("../db/announcements");
const { requireAuth } = require("../middleware/auth");

/**
 * Resolve an appchain by ID — checks listings first, then legacy deployments.
 */
function resolveAppchain(id) {
  const listing = getListingById(id);
  if (listing && listing.status === "active") return listing;
  return getActiveDeploymentById(id) || null;
}

// Middleware: validate appchain exists and is active
function requireAppchain(req, res, next) {
  const appchain = resolveAppchain(req.params.id);
  if (!appchain) return res.status(404).json({ error: "Appchain not found" });
  req.appchain = appchain;
  next();
}

// Middleware: require caller is the appchain owner (must run after requireAppchain + requireWallet)
function requireOwner(req, res, next) {
  const ownerWallet = (req.appchain.owner_wallet || req.appchain.signed_by || "").toLowerCase();
  if (!ownerWallet || req.walletAddress !== ownerWallet) {
    return res.status(403).json({ error: "Only the appchain owner can perform this action" });
  }
  next();
}

// GET /api/store/programs — public program listing
router.get("/programs", (req, res) => {
  try {
    const { category, search, limit, offset } = req.query;
    const programs = getActivePrograms({
      category,
      search,
      limit: parseInt(limit) || 50,
      offset: parseInt(offset) || 0,
    });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/programs/:id — program detail
router.get("/programs/:id", (req, res) => {
  try {
    const program = getProgramById(req.params.id);
    if (!program || program.status !== "active") {
      return res.status(404).json({ error: "Program not found" });
    }
    res.json({ program });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/categories — category list
router.get("/categories", (req, res) => {
  try {
    const categories = getCategories();
    res.json({ categories });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/featured — featured programs (top by usage)
router.get("/featured", (req, res) => {
  try {
    const programs = getActivePrograms({ limit: 6 });
    res.json({ programs });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/appchains — public Open Appchain listing (Showroom)
// Merges listings (from metadata repo) with legacy deployments
router.get("/appchains", (req, res) => {
  try {
    const { search, limit, offset, stack_type, l1_chain_id } = req.query;
    const parsedLimit = parseInt(limit) || 50;
    const parsedOffset = parseInt(offset) || 0;

    // Fetch from both sources (no offset — pagination applied after merge)
    const fetchLimit = parsedLimit + parsedOffset;
    const listings = getListings({
      search, stackType: stack_type, l1ChainId: l1_chain_id,
      limit: fetchLimit, offset: 0,
    });
    const deployments = getActiveDeployments({
      search, limit: fetchLimit, offset: 0,
    });

    // Merge: listings first, then deployments (deduplicate by ID)
    const seenIds = new Set();
    const merged = [];
    for (const item of [...listings, ...deployments]) {
      if (!seenIds.has(item.id)) {
        seenIds.add(item.id);
        merged.push(item);
      }
    }

    // Apply pagination after merge
    const paged = merged.slice(parsedOffset, parsedOffset + parsedLimit);

    // Enrich with social stats
    const ids = paged.map((a) => a.id);
    const stats = getSocialStatsBatch(ids);
    const enriched = paged.map((a) => {
      let hashtags = [];
      try { hashtags = a.hashtags ? JSON.parse(a.hashtags) : []; } catch { /* ignore */ }
      return {
        ...a,
        hashtags,
        avg_rating: stats[a.id]?.avg_rating ?? null,
        review_count: stats[a.id]?.review_count ?? 0,
        comment_count: stats[a.id]?.comment_count ?? 0,
      };
    });

    res.json({ appchains: enriched });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/appchains/:id — public appchain detail (Showroom)
router.get("/appchains/:id", requireAppchain, (req, res) => {
  try {
    const appchain = req.appchain;

    let screenshots = [];
    let social_links = {};
    let l1_contracts = {};
    try { screenshots = appchain.screenshots ? JSON.parse(appchain.screenshots) : []; } catch { /* ignore */ }
    try {
      social_links = appchain.social_links ? JSON.parse(appchain.social_links) : {};
      if (!social_links || typeof social_links !== "object") social_links = {};
      // Also check operator_social_links for listings
      if (appchain.operator_social_links && Object.keys(social_links).length === 0) {
        social_links = JSON.parse(appchain.operator_social_links);
      }
    } catch { /* ignore */ }
    try { l1_contracts = appchain.l1_contracts ? JSON.parse(appchain.l1_contracts) : {}; } catch { /* ignore */ }

    const stats = getSocialStats(appchain.id);

    res.json({
      appchain: { ...appchain, screenshots, social_links, l1_contracts, ...stats },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/store/appchains/:id/rpc-proxy — L2 RPC proxy (CORS bypass)
router.post("/appchains/:id/rpc-proxy", requireAppchain, async (req, res) => {
  try {
    const appchain = req.appchain;
    if (!appchain.rpc_url) {
      return res.status(404).json({ error: "No RPC URL configured for this appchain" });
    }

    // SSRF protection: only allow http(s) URLs, block private/internal IPs
    try {
      const rpcUrl = new URL(appchain.rpc_url);
      if (!["http:", "https:"].includes(rpcUrl.protocol)) {
        return res.status(400).json({ error: "Invalid RPC URL protocol" });
      }
      const host = rpcUrl.hostname;
      if (host === "localhost" || host === "127.0.0.1" || host === "::1" ||
          host.startsWith("10.") || host.startsWith("192.168.") ||
          host.startsWith("169.254.") || host.endsWith(".internal") ||
          /^172\.(1[6-9]|2\d|3[01])\./.test(host)) {
        return res.status(400).json({ error: "RPC URL cannot point to internal addresses" });
      }
    } catch {
      return res.status(400).json({ error: "Invalid RPC URL" });
    }

    const allowedMethods = [
      "eth_blockNumber", "eth_chainId", "eth_gasPrice",
      "ethrex_batchNumber", "ethrex_metadata", "net_version",
    ];

    const { method, params } = req.body;
    if (!method || !allowedMethods.includes(method)) {
      return res.status(400).json({ error: "Method not allowed" });
    }

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);

    const response = await fetch(appchain.rpc_url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: params || [] }),
      signal: controller.signal,
    });

    clearTimeout(timeout);
    const data = await response.json();
    res.json(data);
  } catch (e) {
    console.error(`[rpc-proxy] Error proxying to ${req.params.id}:`, e.message);
    res.status(502).json({ error: "L2 node unreachable" });
  }
});

// ── Social: Reviews ──

// GET /api/store/appchains/:id/reviews — list reviews with reaction counts
router.get("/appchains/:id/reviews", requireAppchain, (req, res) => {
  try {
    const reviews = getReviewsByDeployment(req.params.id);
    const reviewIds = reviews.map((r) => r.id);
    const reactionCounts = getReactionCounts("review", reviewIds);

    const walletAddress = req.headers["x-wallet-address"];
    const userReactions = walletAddress
      ? getUserReactions("review", reviewIds, walletAddress)
      : [];

    res.json({ reviews, reactionCounts, userReactions });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/store/appchains/:id/reviews — create/update review
router.post("/appchains/:id/reviews", requireAppchain, requireWallet, (req, res) => {
  try {
    const { rating, content } = req.body;
    if (!Number.isInteger(rating) || rating < 1 || rating > 5) {
      return res.status(400).json({ error: "Rating must be an integer 1-5" });
    }
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return res.status(400).json({ error: "Content is required" });
    }
    if (content.length > 500) {
      return res.status(400).json({ error: "Content must be 500 characters or less" });
    }

    const review = createReview({
      deploymentId: req.params.id,
      walletAddress: req.walletAddress,
      rating,
      content: content.trim(),
    });
    res.status(201).json({ review });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/store/appchains/:id/reviews/:reviewId — delete own review
router.delete("/appchains/:id/reviews/:reviewId", requireWallet, (req, res) => {
  try {
    const deleted = deleteReview(req.params.reviewId, req.walletAddress);
    if (!deleted) {
      return res.status(404).json({ error: "Not found" });
    }
    res.json({ success: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Social: Comments ──

// GET /api/store/appchains/:id/comments — list comments with reaction counts
router.get("/appchains/:id/comments", requireAppchain, (req, res) => {
  try {
    const comments = getCommentsByDeployment(req.params.id);
    const commentIds = comments.map((c) => c.id);
    const reactionCounts = getReactionCounts("comment", commentIds);

    const walletAddress = req.headers["x-wallet-address"];
    const userReactions = walletAddress
      ? getUserReactions("comment", commentIds, walletAddress)
      : [];

    res.json({ comments, reactionCounts, userReactions });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/store/appchains/:id/comments — create comment
router.post("/appchains/:id/comments", requireAppchain, requireWallet, (req, res) => {
  try {
    const { content, parentId } = req.body;
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return res.status(400).json({ error: "Content is required" });
    }
    if (content.length > 500) {
      return res.status(400).json({ error: "Content must be 500 characters or less" });
    }

    const comment = createComment({
      deploymentId: req.params.id,
      walletAddress: req.walletAddress,
      content: content.trim(),
      parentId: parentId || null,
    });
    res.status(201).json({ comment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/store/appchains/:id/comments/:commentId — delete own comment
router.delete("/appchains/:id/comments/:commentId", requireWallet, (req, res) => {
  try {
    const deleted = deleteComment(req.params.commentId, req.walletAddress);
    if (!deleted) {
      return res.status(404).json({ error: "Not found" });
    }
    res.json({ success: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Social: Reactions ──

// POST /api/store/appchains/:id/reactions — toggle like
router.post("/appchains/:id/reactions", requireAppchain, requireWallet, (req, res) => {
  try {
    const { targetType, targetId } = req.body;
    if (!["review", "comment"].includes(targetType)) {
      return res.status(400).json({ error: "targetType must be 'review' or 'comment'" });
    }
    if (!targetId || typeof targetId !== "string") {
      return res.status(400).json({ error: "targetId is required" });
    }

    // Verify target exists
    if (!targetExists(targetType, targetId)) {
      return res.status(404).json({ error: "Target not found" });
    }

    const result = toggleReaction({
      targetType,
      targetId,
      walletAddress: req.walletAddress,
    });
    res.json(result);
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Bookmarks (account-based) ──

// POST /api/store/appchains/:id/bookmark — toggle bookmark
router.post("/appchains/:id/bookmark", requireAppchain, requireAuth, (req, res) => {
  try {
    const result = toggleBookmark(req.user.id, req.params.id);
    res.json(result);
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/store/bookmarks — list user's bookmarked appchain IDs
router.get("/bookmarks", requireAuth, (req, res) => {
  try {
    const ids = getUserBookmarks(req.user.id);
    res.json({ bookmarks: ids });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Announcements (owner-only) ──

// GET /api/store/appchains/:id/announcements — public
router.get("/appchains/:id/announcements", requireAppchain, (req, res) => {
  try {
    const announcements = getAnnouncements(req.params.id);
    res.json({ announcements });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/store/appchains/:id/announcements — owner wallet only
router.post("/appchains/:id/announcements", requireAppchain, requireWallet, requireOwner, (req, res) => {
  try {
    const { title, content, pinned } = req.body;
    if (!title || typeof title !== "string" || title.trim().length === 0) {
      return res.status(400).json({ error: "Title is required" });
    }
    if (title.length > 100) {
      return res.status(400).json({ error: "Title must be 100 characters or less" });
    }
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return res.status(400).json({ error: "Content is required" });
    }
    if (content.length > 2000) {
      return res.status(400).json({ error: "Content must be 2000 characters or less" });
    }

    const count = getAnnouncementCount(req.params.id);
    if (count >= 10) {
      return res.status(400).json({ error: "Maximum 10 announcements per appchain" });
    }

    const announcement = createAnnouncement({
      deploymentId: req.params.id,
      walletAddress: req.walletAddress,
      title: title.trim(),
      content: content.trim(),
      pinned: !!pinned,
    });
    res.status(201).json({ announcement });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/store/appchains/:id/announcements/:announcementId — owner wallet only
router.put("/appchains/:id/announcements/:announcementId", requireAppchain, requireWallet, requireOwner, (req, res) => {
  try {
    const { title, content, pinned } = req.body;
    if (!title || typeof title !== "string" || title.trim().length === 0) {
      return res.status(400).json({ error: "Title is required" });
    }
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return res.status(400).json({ error: "Content is required" });
    }

    const updated = updateAnnouncement(req.params.announcementId, req.walletAddress, {
      title: title.trim(),
      content: content.trim(),
      pinned: pinned ? 1 : 0,
    });
    if (!updated) {
      return res.status(404).json({ error: "Announcement not found" });
    }
    res.json({ announcement: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/store/appchains/:id/announcements/:announcementId — owner wallet only
router.delete("/appchains/:id/announcements/:announcementId", requireAppchain, requireWallet, requireOwner, (req, res) => {
  try {
    const deleted = deleteAnnouncement(req.params.announcementId, req.walletAddress);
    if (!deleted) {
      return res.status(404).json({ error: "Announcement not found" });
    }
    res.json({ success: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;

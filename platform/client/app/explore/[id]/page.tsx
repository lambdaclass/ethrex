"use client";

import { useState, useEffect, useCallback, useMemo } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import Markdown from "react-markdown";
import { storeApi, socialApi, announcementApi, authApi } from "@/lib/api";
import { L1_NAMES, STACK_LABELS } from "@/lib/constants";
import {
  type ApiWalletSession,
  connectWalletForApi,
  hasWallet,
  shortenAddress,
} from "@/lib/wallet";

interface Announcement {
  id: string;
  deployment_id: string;
  wallet_address: string;
  title: string;
  content: string;
  pinned: number;
  created_at: number;
}

interface AppchainDetail {
  id: string;
  user_id: string;
  name: string;
  description: string | null;
  chain_id: number | null;
  l2_chain_id: number | null;
  rpc_url: string | null;
  status: string;
  phase: string;
  bridge_address: string | null;
  proposer_address: string | null;
  identity_contract: string | null;
  explorer_url: string | null;
  dashboard_url: string | null;
  bridge_url: string | null;
  screenshots: string[];
  social_links: Record<string, string>;
  l1_contracts: Record<string, string>;
  l1_chain_id: number | null;
  network_mode: string | null;
  stack_type: string | null;
  rollup_type: string | null;
  native_token_type: string | null;
  native_token_symbol: string | null;
  native_token_decimals: number | null;
  native_token_l1_address: string | null;
  operator_name: string | null;
  operator_website: string | null;
  owner_wallet: string | null;
  signed_by: string | null;
  program_name: string;
  program_slug: string;
  category: string;
  owner_name: string;
  owner_picture: string | null;
  created_at: number;
  avg_rating: number | null;
  review_count: number;
  comment_count: number;
}

interface LiveStatus {
  blockNumber: number | null;
  batchNumber: number | null;
  gasPrice: string | null;
  online: boolean;
}

interface Review {
  id: string;
  deployment_id: string;
  wallet_address: string;
  rating: number;
  content: string;
  created_at: number;
}

interface Comment {
  id: string;
  deployment_id: string;
  wallet_address: string;
  content: string;
  parent_id: string | null;
  deleted_at: number | null;
  created_at: number;
}


const IPFS_GATEWAY = process.env.NEXT_PUBLIC_IPFS_GATEWAY || "https://gateway.pinata.cloud/ipfs";

function ipfsToHttp(uri: string): string {
  if (uri.startsWith("ipfs://")) {
    return `${IPFS_GATEWAY}/${uri.replace("ipfs://", "")}`;
  }
  return uri;
}

function formatDate(ms: number): string {
  return new Date(ms).toLocaleDateString();
}

function StarIcon({ filled, size = 14 }: { filled: boolean; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24"
      fill={filled ? "#f59e0b" : "none"}
      stroke={filled ? "#f59e0b" : "#d1d5db"}
      strokeWidth="2">
      <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
    </svg>
  );
}

function HeartIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" />
    </svg>
  );
}


export default function AppchainDetailPage() {
  const params = useParams();
  const id = params.id as string;

  const [appchain, setAppchain] = useState<AppchainDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [liveStatus, setLiveStatus] = useState<LiveStatus>({
    blockNumber: null, batchNumber: null, gasPrice: null, online: false,
  });
  const [copied, setCopied] = useState<string | null>(null);

  // Social state
  const [reviews, setReviews] = useState<Review[]>([]);
  const [comments, setComments] = useState<Comment[]>([]);
  const [reactionCounts, setReactionCounts] = useState<Record<string, number>>({});
  const [socialTab, setSocialTab] = useState<"announcements" | "reviews" | "comments">("announcements");
  const [reviewRating, setReviewRating] = useState(5);
  const [reviewText, setReviewText] = useState("");
  const [commentText, setCommentText] = useState("");
  const [replyTo, setReplyTo] = useState<Comment | null>(null);
  const [publishing, setPublishing] = useState(false);
  const [socialError, setSocialError] = useState<string | null>(null);
  const [likedIds, setLikedIds] = useState<Set<string>>(new Set());

  // Wallet session (API auth)
  const [wallet, setWallet] = useState<ApiWalletSession | null>(null);
  const [walletConnecting, setWalletConnecting] = useState(false);

  // Announcements
  const [announcements, setAnnouncements] = useState<Announcement[]>([]);
  const [announcementTitle, setAnnouncementTitle] = useState("");
  const [announcementContent, setAnnouncementContent] = useState("");
  const [announcementPinned, setAnnouncementPinned] = useState(false);
  const [announcementPublishing, setAnnouncementPublishing] = useState(false);
  const [expandedAnnouncement, setExpandedAnnouncement] = useState<string | null>(null);
  const [showAnnouncementForm, setShowAnnouncementForm] = useState(false);
  const [editingAnnouncement, setEditingAnnouncement] = useState<string | null>(null);

  // Owner check: wallet address matches appchain.owner_wallet or signed_by
  const ownerAddr = (appchain?.owner_wallet || appchain?.signed_by || "").toLowerCase();
  const isOwner = !!(wallet && ownerAddr && wallet.address.toLowerCase() === ownerAddr);

  const fetchAnnouncements = useCallback(async () => {
    try {
      const data = await announcementApi.list(id);
      setAnnouncements(data.announcements || []);
    } catch {
      // ignore
    }
  }, [id]);

  useEffect(() => {
    async function load() {
      try {
        const data = await storeApi.appchain(id);
        setAppchain(data);

        // Owner check is done via wallet connection (see wallet effect below)
      } catch {
        setError("Appchain not found");
      } finally {
        setLoading(false);
      }
    }
    load();
    fetchAnnouncements();
  }, [id, fetchAnnouncements]);

  const fetchLiveStatus = useCallback(async () => {
    if (!id) return;
    try {
      const [block, batch, gas] = await Promise.all([
        storeApi.appchainRpc(id, "eth_blockNumber"),
        storeApi.appchainRpc(id, "ethrex_batchNumber"),
        storeApi.appchainRpc(id, "eth_gasPrice"),
      ]);
      const next: LiveStatus = {
        blockNumber: block ? parseInt(block, 16) : null,
        batchNumber: batch ? parseInt(batch, 16) : null,
        gasPrice: gas ? (parseInt(gas, 16) / 1e9).toFixed(4) : null,
        online: block !== null,
      };
      setLiveStatus((prev) =>
        prev.blockNumber === next.blockNumber && prev.batchNumber === next.batchNumber &&
        prev.gasPrice === next.gasPrice && prev.online === next.online ? prev : next
      );
    } catch (err) {
      console.warn("[live-status] Failed to fetch:", err);
      setLiveStatus((prev) => prev.online === false ? prev : { blockNumber: null, batchNumber: null, gasPrice: null, online: false });
    }
  }, [id]);

  useEffect(() => {
    if (!appchain) return;
    fetchLiveStatus();
    const interval = setInterval(fetchLiveStatus, 30000);
    return () => clearInterval(interval);
  }, [appchain, fetchLiveStatus]);

  // Fetch social data from Platform API
  const fetchSocial = useCallback(async () => {
    if (!id) return;
    try {
      const [reviewData, commentData] = await Promise.all([
        socialApi.getReviews(id, wallet?.address),
        socialApi.getComments(id, wallet?.address),
      ]);
      setReviews(reviewData.reviews || []);
      setComments(commentData.comments || []);
      setReactionCounts({
        ...(reviewData.reactionCounts || {}),
        ...(commentData.reactionCounts || {}),
      });
      const liked = new Set<string>([
        ...(reviewData.userReactions || []),
        ...(commentData.userReactions || []),
      ]);
      setLikedIds(liked);
    } catch (err) {
      console.warn("[social] Failed to fetch:", err);
    }
  }, [id, wallet?.address]);

  useEffect(() => {
    fetchSocial();
  }, [fetchSocial]);

  const handleConnectWallet = async () => {
    setWalletConnecting(true);
    setSocialError(null);
    try {
      const session = await connectWalletForApi();
      setWallet(session);
    } catch (err) {
      setSocialError(err instanceof Error ? err.message : "Failed to connect wallet");
    } finally {
      setWalletConnecting(false);
    }
  };

  const handleDisconnect = () => {
    setWallet(null);
  };

  const handlePublishReview = async () => {
    if (!reviewText.trim() || !wallet) return;
    setPublishing(true);
    setSocialError(null);
    try {
      await socialApi.createReview(id, { rating: reviewRating, content: reviewText.trim() }, wallet);
      setReviewText("");
      setReviewRating(5);
      await fetchSocial();
    } catch (err) {
      setSocialError(err instanceof Error ? err.message : "Failed to publish review");
    } finally {
      setPublishing(false);
    }
  };

  const handlePublishComment = async () => {
    if (!commentText.trim() || !wallet) return;
    setPublishing(true);
    setSocialError(null);
    try {
      await socialApi.createComment(id, { content: commentText.trim(), parentId: replyTo?.id }, wallet);
      setCommentText("");
      setReplyTo(null);
      await fetchSocial();
    } catch (err) {
      setSocialError(err instanceof Error ? err.message : "Failed to publish comment");
    } finally {
      setPublishing(false);
    }
  };

  const handleLike = async (targetType: "review" | "comment", targetId: string) => {
    if (!wallet) return;
    try {
      const result = await socialApi.toggleReaction(id, { targetType, targetId }, wallet);
      if (result.liked) {
        setLikedIds((prev) => new Set(prev).add(targetId));
      } else {
        setLikedIds((prev) => {
          const next = new Set(prev);
          next.delete(targetId);
          return next;
        });
      }
      setReactionCounts((prev) => ({ ...prev, [targetId]: result.count }));
    } catch (err) {
      console.warn("[social] Failed to toggle reaction:", err);
    }
  };

  const handleDeleteReview = async (reviewId: string) => {
    if (!wallet) return;
    try {
      await socialApi.deleteReview(id, reviewId, wallet);
      await fetchSocial();
    } catch (err) {
      console.warn("[social] Failed to delete review:", err);
    }
  };

  const handleDeleteComment = async (commentId: string) => {
    if (!wallet) return;
    try {
      await socialApi.deleteComment(id, commentId, wallet);
      await fetchSocial();
    } catch (err) {
      console.warn("[social] Failed to delete comment:", err);
    }
  };

  const handlePostAnnouncement = async () => {
    if (!announcementTitle.trim() || !announcementContent.trim() || !wallet) return;
    setAnnouncementPublishing(true);
    try {
      await announcementApi.create(id, { title: announcementTitle.trim(), content: announcementContent.trim(), pinned: announcementPinned }, wallet);
      setAnnouncementTitle("");
      setAnnouncementContent("");
      setAnnouncementPinned(false);
      setShowAnnouncementForm(false);
      await fetchAnnouncements();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to post";
      setSocialError(msg);
      console.warn("[announcements] Failed to post:", err);
    } finally {
      setAnnouncementPublishing(false);
    }
  };

  const handleUpdateAnnouncement = async () => {
    if (!editingAnnouncement || !announcementTitle.trim() || !announcementContent.trim() || !wallet) return;
    setAnnouncementPublishing(true);
    try {
      await announcementApi.update(id, editingAnnouncement, { title: announcementTitle.trim(), content: announcementContent.trim(), pinned: announcementPinned }, wallet);
      setEditingAnnouncement(null);
      setAnnouncementTitle("");
      setAnnouncementContent("");
      setAnnouncementPinned(false);
      await fetchAnnouncements();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to update";
      setSocialError(msg);
    } finally {
      setAnnouncementPublishing(false);
    }
  };

  const handleDeleteAnnouncement = async (announcementId: string) => {
    if (!wallet) return;
    try {
      await announcementApi.delete(id, announcementId, wallet);
      await fetchAnnouncements();
    } catch (err) {
      console.warn("[announcements] Failed to delete:", err);
    }
  };

  const copyToClipboard = (text: string, label: string) => {
    navigator.clipboard.writeText(text);
    setCopied(label);
    setTimeout(() => setCopied(null), 2000);
  };

  const pinnedAnnouncements = useMemo(() => announcements.filter((a) => a.pinned), [announcements]);

  // Pre-group comments: top-level + replies map
  const { topLevelComments, repliesByParent } = useMemo(() => {
    const repliesMap = new Map<string, Comment[]>();
    const topLevel: Comment[] = [];
    for (const c of comments) {
      if (c.parent_id) {
        const list = repliesMap.get(c.parent_id) || [];
        list.push(c);
        repliesMap.set(c.parent_id, list);
      } else {
        topLevel.push(c);
      }
    }
    const filtered = topLevel.filter((c) => !c.deleted_at || (repliesMap.get(c.id)?.length ?? 0) > 0);
    return { topLevelComments: filtered, repliesByParent: repliesMap };
  }, [comments]);

  if (loading) {
    return (
      <div className="flex justify-center py-32">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
      </div>
    );
  }

  if (error || !appchain) {
    return (
      <div className="max-w-4xl mx-auto px-4 py-16 text-center">
        <h2 className="text-xl font-semibold text-gray-700 mb-2">Appchain not found</h2>
        <Link href="/explore" className="text-blue-600 hover:underline">Back to Explore</Link>
      </div>
    );
  }

  const l1Name = appchain.l1_chain_id ? (L1_NAMES[appchain.l1_chain_id] || `Chain ${appchain.l1_chain_id}`) : null;
  const stackLabel = appchain.stack_type ? STACK_LABELS[appchain.stack_type] || appchain.stack_type : null;
  const chainId = appchain.l2_chain_id || appchain.chain_id;
  const displayName = appchain.operator_name || appchain.owner_name;

  // Build contracts list from l1_contracts (listings) or legacy fields
  const contracts = appchain.l1_contracts && Object.keys(appchain.l1_contracts).length > 0
    ? Object.entries(appchain.l1_contracts).map(([label, addr]) => ({ label, addr }))
    : [
        { label: "OnChainProposer", addr: appchain.proposer_address },
        { label: "CommonBridge", addr: appchain.bridge_address },
      ].filter((c) => c.addr);

  const isSafeUrl = (url: string) => /^https?:\/\//i.test(url);
  const socialEntries = Object.entries(appchain.social_links || {}).filter(([, v]) => v && isSafeUrl(v));

  const isMyReview = (review: Review) =>
    wallet && review.wallet_address.toLowerCase() === wallet.address.toLowerCase();
  const isMyComment = (comment: Comment) =>
    wallet && comment.wallet_address.toLowerCase() === wallet.address.toLowerCase();

  const hasServices = !!(appchain.explorer_url || appchain.dashboard_url || appchain.bridge_url || appchain.rpc_url);

  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      {/* Back link */}
      <Link href="/explore" className="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 mb-6">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="15 18 9 12 15 6" />
        </svg>
        Back to Explore
      </Link>

      {/* Header */}
      <div className="bg-white rounded-xl border p-6 mb-4">
        <div className="flex items-start justify-between">
          <div>
            <h1 className="text-2xl font-bold">{appchain.name}</h1>
            <p className="text-gray-500 mt-1">
              by {displayName}
              {stackLabel && <span className="ml-2 text-xs px-2 py-0.5 bg-indigo-50 text-indigo-700 rounded">{stackLabel}</span>}
              {l1Name && <span className="ml-2 text-xs px-2 py-0.5 bg-purple-50 text-purple-700 rounded">{l1Name}</span>}
              {chainId && <span className="ml-2 text-xs font-mono text-gray-400">Chain ID: {chainId}</span>}
            </p>
            {/* Rating summary */}
            {appchain.avg_rating !== null && (
              <div className="flex items-center gap-2 mt-2">
                <div className="flex gap-0.5">
                  {[1, 2, 3, 4, 5].map((star) => (
                    <StarIcon key={star} filled={star <= Math.round(appchain.avg_rating!)} size={16} />
                  ))}
                </div>
                <span className="text-sm font-medium text-gray-700">{appchain.avg_rating}</span>
                <span className="text-sm text-gray-400">({appchain.review_count} review{appchain.review_count !== 1 ? "s" : ""})</span>
              </div>
            )}
          </div>
          <div className="flex items-center gap-2">
            {liveStatus.online ? (
              <span className="flex items-center gap-1.5 text-sm text-green-600">
                <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                Online
              </span>
            ) : (
              <span className="flex items-center gap-1.5 text-sm text-gray-400">
                <span className="w-2 h-2 rounded-full bg-gray-300" />
                Offline
              </span>
            )}
            <span className="px-2 py-0.5 bg-gray-100 text-gray-600 rounded text-xs">{appchain.category}</span>
          </div>
        </div>
        {/* Add to Wallet */}
        {chainId && appchain.rpc_url && (
          <button
            onClick={async () => {
              try {
                const eth = (window as unknown as { ethereum?: { request: (args: unknown) => Promise<unknown> } }).ethereum;
                if (!eth) return;
                const tokenSymbol = appchain.native_token_symbol || "TON";
                const tokenDecimals = appchain.native_token_decimals ?? 18;
                await eth.request({
                  method: "wallet_addEthereumChain",
                  params: [{
                    chainId: `0x${chainId.toString(16)}`,
                    chainName: appchain.name,
                    nativeCurrency: { name: tokenSymbol, symbol: tokenSymbol, decimals: tokenDecimals },
                    rpcUrls: [appchain.rpc_url],
                    blockExplorerUrls: appchain.explorer_url ? [appchain.explorer_url] : undefined,
                  }],
                });
              } catch {
                // User rejected or no wallet
              }
            }}
            className="mt-3 w-full py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700 transition-colors flex items-center justify-center gap-2"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 12V7H5a2 2 0 0 1 0-4h14v4"/><path d="M3 5v14a2 2 0 0 0 2 2h16v-5"/><path d="M18 12a2 2 0 0 0 0 4h4v-4Z"/>
            </svg>
            Add to Wallet
          </button>
        )}
      </div>

      {/* Pinned announcements — below header, click to expand in Community */}
      {pinnedAnnouncements.length > 0 && (
        <div className="mb-4 space-y-1.5">
          {pinnedAnnouncements.map((a) => (
            <button
              key={a.id}
              onClick={() => {
                setSocialTab("announcements");
                setExpandedAnnouncement(a.id);
                document.getElementById("community-section")?.scrollIntoView({ behavior: "smooth" });
              }}
              className="w-full bg-blue-50 border border-blue-100 rounded-lg px-3 py-2 flex items-center gap-2 text-sm hover:bg-blue-100 transition-colors text-left"
            >
              <span className="flex-shrink-0">📌</span>
              <span className="font-medium text-blue-900 truncate">{a.title}</span>
              <span className="text-xs text-blue-400 ml-auto flex-shrink-0">{formatDate(a.created_at)}</span>
            </button>
          ))}
        </div>
      )}

      {/* Description */}
      {appchain.description && (
        <div className="bg-white rounded-xl border p-6 mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">About</h2>
          <p className="text-gray-700 whitespace-pre-wrap">{appchain.description}</p>
        </div>
      )}

      {/* Appchain Details (listing-specific) */}
      {(appchain.native_token_symbol || appchain.operator_name || appchain.rollup_type) && (
        <div className="bg-white rounded-xl border p-6 mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">Details</h2>
          <div className="grid grid-cols-2 gap-x-6 gap-y-3">
            {appchain.rollup_type && (
              <div>
                <span className="text-xs text-gray-500">Rollup Type</span>
                <p className="text-sm font-medium">{appchain.rollup_type}</p>
              </div>
            )}
            {appchain.native_token_symbol && (
              <div>
                <span className="text-xs text-gray-500">Native Token</span>
                <p className="text-sm font-medium">
                  {appchain.native_token_symbol}
                  {appchain.native_token_type === "erc20" && (
                    <span className="ml-1 text-xs text-gray-400">(ERC-20)</span>
                  )}
                </p>
              </div>
            )}
            {appchain.operator_name && (
              <div>
                <span className="text-xs text-gray-500">Operator</span>
                <p className="text-sm font-medium">
                  {appchain.operator_website ? (
                    <a href={appchain.operator_website} target="_blank" rel="noopener noreferrer" className="text-blue-600 hover:underline">
                      {appchain.operator_name}
                    </a>
                  ) : appchain.operator_name}
                </p>
              </div>
            )}
            {appchain.signed_by && (
              <div>
                <span className="text-xs text-gray-500">Signed By</span>
                <p className="text-sm font-mono text-gray-600">{shortenAddress(appchain.signed_by)}</p>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Screenshots */}
      {appchain.screenshots.length > 0 && (
        <div className="bg-white rounded-xl border p-6 mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">Screenshots</h2>
          <div className="flex gap-3 overflow-x-auto pb-2">
            {appchain.screenshots.map((uri, i) => (
              <img
                key={i}
                src={ipfsToHttp(uri)}
                alt={`Screenshot ${i + 1}`}
                className="h-40 rounded-lg border object-cover flex-shrink-0"
              />
            ))}
          </div>
        </div>
      )}

      {/* Network Info — consolidated card (hidden if nothing to show) */}
      {(liveStatus.online || hasServices || contracts.length > 0 || socialEntries.length > 0) && (
      <div className="bg-white rounded-xl border p-6 mb-4">
        {/* Live Status — only show when online */}
        {liveStatus.online && (
          <>
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide">Network</h2>
              <button onClick={fetchLiveStatus} className="text-xs text-blue-600 hover:underline">Refresh</button>
            </div>
            <div className="grid grid-cols-3 gap-3 mb-4">
              <div className="bg-gray-50 rounded-lg px-3 py-2 text-center">
                <div className="text-xs text-gray-500">Block</div>
                <div className="text-base font-semibold font-mono">{liveStatus.blockNumber?.toLocaleString()}</div>
              </div>
              <div className="bg-gray-50 rounded-lg px-3 py-2 text-center">
                <div className="text-xs text-gray-500">Batch</div>
                <div className="text-base font-semibold font-mono">{liveStatus.batchNumber?.toLocaleString() ?? "\u2014"}</div>
              </div>
              <div className="bg-gray-50 rounded-lg px-3 py-2 text-center">
                <div className="text-xs text-gray-500">Gas Price</div>
                <div className="text-base font-semibold font-mono">{liveStatus.gasPrice ? `${liveStatus.gasPrice} Gwei` : "\u2014"}</div>
              </div>
            </div>
          </>
        )}

        {/* Services + RPC */}
        {hasServices && (
          <div className={`space-y-2 ${liveStatus.online ? "border-t pt-4 mb-4" : "mb-4"}`}>
            {!liveStatus.online && (
              <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">Network</h2>
            )}
            {appchain.explorer_url && isSafeUrl(appchain.explorer_url) && (
              <div className="flex items-center justify-between">
                <span className="text-sm text-gray-600">L2 Explorer</span>
                <a href={appchain.explorer_url} target="_blank" rel="noopener noreferrer"
                  className="text-sm text-blue-600 hover:underline flex items-center gap-1 truncate max-w-[60%]">
                  {appchain.explorer_url.replace(/^https?:\/\//, "").slice(0, 40)}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
                </a>
              </div>
            )}
            {appchain.dashboard_url && isSafeUrl(appchain.dashboard_url) && (
              <div className="flex items-center justify-between">
                <span className="text-sm text-gray-600">Dashboard</span>
                <a href={appchain.dashboard_url} target="_blank" rel="noopener noreferrer"
                  className="text-sm text-blue-600 hover:underline flex items-center gap-1 truncate max-w-[60%]">
                  {appchain.dashboard_url.replace(/^https?:\/\//, "").slice(0, 40)}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
                </a>
              </div>
            )}
            {appchain.bridge_url && isSafeUrl(appchain.bridge_url) && (
              <div className="flex items-center justify-between">
                <span className="text-sm text-gray-600">Bridge</span>
                <a href={appchain.bridge_url} target="_blank" rel="noopener noreferrer"
                  className="text-sm text-blue-600 hover:underline flex items-center gap-1 truncate max-w-[60%]">
                  {appchain.bridge_url.replace(/^https?:\/\//, "").slice(0, 40)}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
                </a>
              </div>
            )}
            {appchain.rpc_url && (
              <div className="flex items-center justify-between">
                <span className="text-sm text-gray-600">RPC URL</span>
                <button onClick={() => copyToClipboard(appchain.rpc_url!, "rpc")}
                  className="text-sm font-mono text-gray-700 hover:text-blue-600 flex items-center gap-1 truncate max-w-[60%]">
                  {appchain.rpc_url.length > 40 ? appchain.rpc_url.slice(0, 40) + "..." : appchain.rpc_url}
                  <span className="text-xs text-gray-400">{copied === "rpc" ? "Copied!" : ""}</span>
                </button>
              </div>
            )}
          </div>
        )}

        {/* L1 Contracts */}
        {contracts.length > 0 && (
          <div className={`${liveStatus.online || hasServices ? "border-t pt-4" : ""} mb-4`}>
            {!liveStatus.online && !hasServices && (
              <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">Network</h2>
            )}
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
              L1 Contracts{l1Name && ` (${l1Name})`}
            </h3>
            <div className="grid grid-cols-2 gap-2">
              {contracts.map((c) => (
                <div key={c.label} className="flex items-center justify-between bg-gray-50 rounded-lg px-3 py-2">
                  <div>
                    <div className="text-xs text-gray-500">{c.label}</div>
                    <div className="text-sm font-mono">{shortenAddress(c.addr!)}</div>
                  </div>
                  <button onClick={() => copyToClipboard(c.addr!, c.label)}
                    className="text-gray-400 hover:text-gray-600 p-1">
                    {copied === c.label ? (
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polyline points="20 6 9 17 4 12"/></svg>
                    ) : (
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
                    )}
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Social Links */}
        {socialEntries.length > 0 && (
          <div className={`${liveStatus.online || hasServices || contracts.length > 0 ? "border-t pt-4" : ""}`}>
            <div className="flex gap-2 flex-wrap">
              {socialEntries.map(([key, url]) => (
                <a key={key} href={url} target="_blank" rel="noopener noreferrer"
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-gray-50 rounded-lg text-sm text-gray-700 hover:bg-gray-100 border">
                  {key.charAt(0).toUpperCase() + key.slice(1)}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
                </a>
              ))}
            </div>
          </div>
        )}
      </div>
      )}

      {/* Community (Platform DB Social) */}
      <div id="community-section" className="bg-white rounded-xl border p-6 mb-4">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide">Community</h2>
          {/* Wallet Connection */}
          {wallet ? (
            <div className="flex items-center gap-2">
              <span className="text-xs font-mono text-gray-600 bg-gray-100 px-2 py-1 rounded">
                {shortenAddress(wallet.address)}
              </span>
              <button
                onClick={handleDisconnect}
                className="text-xs text-gray-400 hover:text-gray-600"
              >
                Disconnect
              </button>
            </div>
          ) : (
            <button
              onClick={handleConnectWallet}
              disabled={walletConnecting}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-gray-900 text-white text-xs font-medium rounded-lg hover:bg-gray-800 disabled:opacity-50 transition-colors"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M21 12V7H5a2 2 0 0 1 0-4h14v4"/><path d="M3 5v14a2 2 0 0 0 2 2h16v-5"/><path d="M18 12a2 2 0 0 0 0 4h4v-4Z"/>
              </svg>
              {walletConnecting ? "Connecting..." : "Sign in with Wallet"}
            </button>
          )}
        </div>

        {/* Tabs */}
        <div className="flex gap-1 bg-gray-100 rounded-lg p-1 mb-4">
          <button
            onClick={() => setSocialTab("announcements")}
            className={`flex-1 py-1.5 text-sm font-medium rounded-md transition-colors ${
              socialTab === "announcements" ? "bg-white shadow-sm text-gray-900" : "text-gray-500 hover:text-gray-700"
            }`}
          >
            Announcements {announcements.length > 0 && `(${announcements.length})`}
          </button>
          <button
            onClick={() => setSocialTab("reviews")}
            className={`flex-1 py-1.5 text-sm font-medium rounded-md transition-colors ${
              socialTab === "reviews" ? "bg-white shadow-sm text-gray-900" : "text-gray-500 hover:text-gray-700"
            }`}
          >
            Reviews ({reviews.length})
          </button>
          <button
            onClick={() => setSocialTab("comments")}
            className={`flex-1 py-1.5 text-sm font-medium rounded-md transition-colors ${
              socialTab === "comments" ? "bg-white shadow-sm text-gray-900" : "text-gray-500 hover:text-gray-700"
            }`}
          >
            Comments ({comments.length})
          </button>
        </div>

        {socialError && (
          <div className="text-sm text-red-600 bg-red-50 rounded-lg px-3 py-2 mb-3">{socialError}</div>
        )}

        {/* Announcements Tab — bulletin board style */}
        {socialTab === "announcements" && (
          <div>
            {announcements.length === 0 && !showAnnouncementForm ? (
              <p className="text-center text-gray-400 text-sm py-6">No announcements yet.</p>
            ) : (
              <div className="border rounded-lg divide-y">
                {announcements.map((a) => (
                  <div key={a.id}>
                    <button
                      onClick={() => setExpandedAnnouncement(expandedAnnouncement === a.id ? null : a.id)}
                      className="w-full px-4 py-3 flex items-center gap-3 text-left hover:bg-gray-50 transition-colors"
                    >
                      {a.pinned ? <span className="text-xs flex-shrink-0">📌</span> : null}
                      <span className="text-sm font-medium text-gray-900 flex-1 truncate">{a.title}</span>
                      <span className="text-xs text-gray-400 flex-shrink-0">{formatDate(a.created_at)}</span>
                      <svg
                        width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
                        className={`flex-shrink-0 text-gray-400 transition-transform ${expandedAnnouncement === a.id ? "rotate-180" : ""}`}
                      >
                        <polyline points="6 9 12 15 18 9" />
                      </svg>
                    </button>
                    {expandedAnnouncement === a.id && (
                      editingAnnouncement === a.id ? (
                        <div className="px-4 pb-3 bg-gray-50">
                          <input
                            value={announcementTitle}
                            onChange={(e) => setAnnouncementTitle(e.target.value)}
                            className="w-full border rounded-lg px-3 py-2 text-sm mb-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                            maxLength={100}
                          />
                          <textarea
                            value={announcementContent}
                            onChange={(e) => setAnnouncementContent(e.target.value)}
                            className="w-full border rounded-lg px-3 py-2 text-sm resize-none h-32 focus:outline-none focus:ring-2 focus:ring-blue-500"
                            maxLength={2000}
                          />
                          <div className="flex items-center justify-between mt-2">
                            <label className="flex items-center gap-1.5 text-xs text-gray-500 cursor-pointer">
                              <input type="checkbox" checked={announcementPinned} onChange={(e) => setAnnouncementPinned(e.target.checked)} className="rounded" />
                              Pin to top
                            </label>
                            <div className="flex items-center gap-2">
                              <button onClick={() => { setEditingAnnouncement(null); setAnnouncementTitle(""); setAnnouncementContent(""); }} className="px-3 py-1 text-xs text-gray-500">Cancel</button>
                              <button onClick={handleUpdateAnnouncement} disabled={announcementPublishing || !announcementTitle.trim() || !announcementContent.trim()} className="px-3 py-1 bg-blue-600 text-white text-xs rounded-lg disabled:opacity-50">{announcementPublishing ? "..." : "Save"}</button>
                            </div>
                          </div>
                        </div>
                      ) : (
                        <div className="px-4 pb-3 bg-gray-50">
                          <div className="prose prose-sm max-w-none text-gray-700">
                            <Markdown>{a.content}</Markdown>
                          </div>
                          <div className="flex items-center justify-between mt-2">
                            <span className="text-xs font-mono text-gray-400">{shortenAddress(a.wallet_address)}</span>
                            {isOwner && (
                              <div className="flex items-center gap-2">
                                <button
                                  onClick={() => { setEditingAnnouncement(a.id); setAnnouncementTitle(a.title); setAnnouncementContent(a.content); setAnnouncementPinned(!!a.pinned); }}
                                  className="text-xs text-blue-500 hover:text-blue-700"
                                >Edit</button>
                                <button onClick={() => handleDeleteAnnouncement(a.id)} className="text-xs text-red-400 hover:text-red-600">Delete</button>
                              </div>
                            )}
                          </div>
                        </div>
                      )
                    )}
                  </div>
                ))}
              </div>
            )}

            {/* Write Announcement — owner only */}
            {isOwner && announcements.length < 10 && (
              showAnnouncementForm ? (
                <div className="border rounded-lg p-4 mt-4 bg-gray-50">
                  <input
                    value={announcementTitle}
                    onChange={(e) => setAnnouncementTitle(e.target.value)}
                    placeholder="Title"
                    className="w-full border rounded-lg px-3 py-2 text-sm mb-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                    maxLength={100}
                  />
                  <textarea
                    value={announcementContent}
                    onChange={(e) => setAnnouncementContent(e.target.value)}
                    placeholder="Content..."
                    className="w-full border rounded-lg px-3 py-2 text-sm resize-none h-24 focus:outline-none focus:ring-2 focus:ring-blue-500"
                    maxLength={2000}
                  />
                  <div className="flex items-center justify-between mt-2">
                    <label className="flex items-center gap-1.5 text-xs text-gray-500 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={announcementPinned}
                        onChange={(e) => setAnnouncementPinned(e.target.checked)}
                        className="rounded"
                      />
                      Pin to top of page
                    </label>
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => { setShowAnnouncementForm(false); setAnnouncementTitle(""); setAnnouncementContent(""); }}
                        className="px-3 py-1.5 text-sm text-gray-500 hover:text-gray-700"
                      >
                        Cancel
                      </button>
                      <button
                        onClick={handlePostAnnouncement}
                        disabled={announcementPublishing || !announcementTitle.trim() || !announcementContent.trim()}
                        className="px-4 py-1.5 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                      >
                        {announcementPublishing ? "Posting..." : "Post"}
                      </button>
                    </div>
                  </div>
                </div>
              ) : (
                <button
                  onClick={() => setShowAnnouncementForm(true)}
                  className="mt-4 w-full py-2 border border-dashed rounded-lg text-sm text-gray-500 hover:text-blue-600 hover:border-blue-300 transition-colors"
                >
                  + Write Announcement
                </button>
              )
            )}
          </div>
        )}

        {/* Reviews Tab */}
        {socialTab === "reviews" && (
          <div>
            {/* Write Review */}
            {wallet ? (
              <div className="border rounded-lg p-4 mb-4 bg-gray-50">
                <div className="flex items-center gap-2 mb-2">
                  <span className="text-sm text-gray-600">Rating:</span>
                  <div className="flex gap-0.5">
                    {[1, 2, 3, 4, 5].map((star) => (
                      <button key={star} onClick={() => setReviewRating(star)} className="p-0.5">
                        <StarIcon filled={star <= reviewRating} size={20} />
                      </button>
                    ))}
                  </div>
                </div>
                <textarea
                  value={reviewText}
                  onChange={(e) => setReviewText(e.target.value)}
                  placeholder="Share your experience with this appchain..."
                  className="w-full border rounded-lg px-3 py-2 text-sm resize-none h-20 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  maxLength={500}
                />
                <div className="flex items-center justify-between mt-2">
                  <span className="text-xs text-gray-400">
                    Posting as {shortenAddress(wallet.address)}
                  </span>
                  <button
                    onClick={handlePublishReview}
                    disabled={publishing || !reviewText.trim()}
                    className="px-4 py-1.5 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {publishing ? "Publishing..." : "Post Review"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="border border-dashed rounded-lg p-6 mb-4 text-center text-gray-400">
                <p className="text-sm">
                  {hasWallet()
                    ? "Connect your wallet to write a review"
                    : "Install MetaMask to write a review"}
                </p>
              </div>
            )}

            {/* Review List */}
            {reviews.length === 0 ? (
              <p className="text-center text-gray-400 text-sm py-6">No reviews yet. Be the first!</p>
            ) : (
              <div className="space-y-3">
                {reviews.map((review) => (
                  <div key={review.id} className="border rounded-lg p-4">
                    <div className="flex items-center justify-between mb-2">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-mono text-gray-500" title={review.wallet_address}>
                          {shortenAddress(review.wallet_address)}
                        </span>
                        <div className="flex gap-0.5">
                          {[1, 2, 3, 4, 5].map((star) => (
                            <StarIcon key={star} filled={star <= review.rating} />
                          ))}
                        </div>
                      </div>
                      <div className="flex items-center gap-2">
                        <span className="text-xs text-gray-400">
                          {formatDate(review.created_at)}
                        </span>
                        {isMyReview(review) && (
                          <button
                            onClick={() => handleDeleteReview(review.id)}
                            className="text-xs text-red-400 hover:text-red-600"
                            title="Delete your review"
                          >
                            Delete
                          </button>
                        )}
                      </div>
                    </div>
                    <p className="text-sm text-gray-700 whitespace-pre-wrap">{review.content}</p>
                    <div className="mt-2 flex items-center gap-1">
                      <button
                        onClick={() => handleLike("review", review.id)}
                        disabled={!wallet}
                        className={`flex items-center gap-1 text-xs transition-colors ${
                          likedIds.has(review.id)
                            ? "text-red-500"
                            : "text-gray-400 hover:text-red-500"
                        } disabled:hover:text-gray-400 disabled:cursor-not-allowed`}
                      >
                        <HeartIcon />
                        {reactionCounts[review.id] || 0}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Comments Tab */}
        {socialTab === "comments" && (
          <div>
            {/* Comment List */}
            {comments.length === 0 ? (
              <p className="text-center text-gray-400 text-sm py-6">No comments yet. Start the conversation!</p>
            ) : (
              <div className="space-y-3 max-h-[500px] overflow-y-auto">
                {topLevelComments.map((comment) => {
                    const replies = repliesByParent.get(comment.id) || [];
                    return (
                      <div key={comment.id}>
                        {/* Parent comment */}
                        {comment.deleted_at ? (
                          <div className="border rounded-lg p-4 bg-gray-50">
                            <p className="text-sm text-gray-400 italic">This comment has been deleted</p>
                          </div>
                        ) : (
                          <div className="border rounded-lg p-4">
                            <div className="flex items-center justify-between mb-2">
                              <span className="text-xs font-mono text-gray-500" title={comment.wallet_address}>
                                {shortenAddress(comment.wallet_address)}
                              </span>
                              <div className="flex items-center gap-2">
                                <span className="text-xs text-gray-400">
                                  {formatDate(comment.created_at)}
                                </span>
                                {isMyComment(comment) && (
                                  <button
                                    onClick={() => handleDeleteComment(comment.id)}
                                    className="text-xs text-red-400 hover:text-red-600"
                                    title="Delete your comment"
                                  >
                                    Delete
                                  </button>
                                )}
                              </div>
                            </div>
                            <p className="text-sm text-gray-700 whitespace-pre-wrap">{comment.content}</p>
                            <div className="mt-2 flex items-center gap-3">
                              <button
                                onClick={() => handleLike("comment", comment.id)}
                                disabled={!wallet}
                                className={`flex items-center gap-1 text-xs transition-colors ${
                                  likedIds.has(comment.id)
                                    ? "text-red-500"
                                    : "text-gray-400 hover:text-red-500"
                                } disabled:hover:text-gray-400 disabled:cursor-not-allowed`}
                              >
                                <HeartIcon />
                                {reactionCounts[comment.id] || 0}
                              </button>
                              {wallet && (
                                <button
                                  onClick={() => setReplyTo(replyTo?.id === comment.id ? null : comment)}
                                  className={`text-xs transition-colors ${
                                    replyTo?.id === comment.id ? "text-blue-600 font-medium" : "text-gray-400 hover:text-blue-600"
                                  }`}
                                >
                                  {replyTo?.id === comment.id ? "Cancel" : "Reply"}
                                </button>
                              )}
                            </div>
                          </div>
                        )}

                        {/* Replies (indented) */}
                        {replies.length > 0 && (
                          <div className="ml-6 mt-2 space-y-2 border-l-2 border-gray-100 pl-4">
                            {replies.map((reply) => (
                              <div key={reply.id} className="border rounded-lg p-3 bg-gray-50">
                                {reply.deleted_at ? (
                                  <p className="text-sm text-gray-400 italic">This comment has been deleted</p>
                                ) : (
                                  <>
                                    <div className="flex items-center justify-between mb-1">
                                      <span className="text-xs font-mono text-gray-500" title={reply.wallet_address}>
                                        {shortenAddress(reply.wallet_address)}
                                      </span>
                                      <div className="flex items-center gap-2">
                                        <span className="text-xs text-gray-400">
                                          {formatDate(reply.created_at)}
                                        </span>
                                        {isMyComment(reply) && (
                                          <button
                                            onClick={() => handleDeleteComment(reply.id)}
                                            className="text-xs text-red-400 hover:text-red-600"
                                            title="Delete your reply"
                                          >
                                            Delete
                                          </button>
                                        )}
                                      </div>
                                    </div>
                                    <p className="text-sm text-gray-700 whitespace-pre-wrap">{reply.content}</p>
                                    <div className="mt-1 flex items-center gap-3">
                                      <button
                                        onClick={() => handleLike("comment", reply.id)}
                                        disabled={!wallet}
                                        className={`flex items-center gap-1 text-xs transition-colors ${
                                          likedIds.has(reply.id)
                                            ? "text-red-500"
                                            : "text-gray-400 hover:text-red-500"
                                        } disabled:hover:text-gray-400 disabled:cursor-not-allowed`}
                                      >
                                        <HeartIcon />
                                        {reactionCounts[reply.id] || 0}
                                      </button>
                                      {wallet && (
                                        <button
                                          onClick={() => setReplyTo(comment)}
                                          className="text-xs text-gray-400 hover:text-blue-600 transition-colors"
                                        >
                                          Reply
                                        </button>
                                      )}
                                    </div>
                                  </>
                                )}
                              </div>
                            ))}
                          </div>
                        )}

                        {/* Inline reply input — appears inside this comment when Reply is clicked */}
                        {replyTo?.id === comment.id && wallet && (
                          <div className="ml-6 mt-2 border-l-2 border-blue-200 pl-4">
                            <div className="border rounded-lg p-3 bg-blue-50/50">
                              <textarea
                                value={commentText}
                                onChange={(e) => setCommentText(e.target.value)}
                                placeholder={`Reply to ${shortenAddress(comment.wallet_address)}...`}
                                className="w-full border rounded-lg px-3 py-2 text-sm resize-none h-14 focus:outline-none focus:ring-2 focus:ring-blue-500 bg-white"
                                maxLength={500}
                                autoFocus
                              />
                              <div className="flex items-center justify-between mt-2">
                                <span className="text-xs text-gray-400">
                                  {shortenAddress(wallet.address)}
                                </span>
                                <div className="flex items-center gap-2">
                                  <button
                                    onClick={() => { setReplyTo(null); setCommentText(""); }}
                                    className="px-3 py-1 text-xs text-gray-500 hover:text-gray-700 transition-colors"
                                  >
                                    Cancel
                                  </button>
                                  <button
                                    onClick={handlePublishComment}
                                    disabled={publishing || !commentText.trim()}
                                    className="px-3 py-1 bg-blue-600 text-white text-xs rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                                  >
                                    {publishing ? "..." : "Reply"}
                                  </button>
                                </div>
                              </div>
                            </div>
                          </div>
                        )}
                      </div>
                    );
                  })}
              </div>
            )}

            {/* Write Comment — at the bottom */}
            {wallet ? (
              <div className="border rounded-lg p-4 mt-4 bg-gray-50">
                <textarea
                  value={replyTo ? "" : commentText}
                  onChange={(e) => { if (!replyTo) setCommentText(e.target.value); }}
                  placeholder="Leave a comment..."
                  className={`w-full border rounded-lg px-3 py-2 text-sm resize-none h-16 focus:outline-none focus:ring-2 focus:ring-blue-500 ${replyTo ? "opacity-50 cursor-not-allowed" : ""}`}
                  maxLength={500}
                  disabled={!!replyTo}
                />
                <div className="flex items-center justify-between mt-2">
                  <span className="text-xs text-gray-400">
                    Posting as {shortenAddress(wallet.address)}
                  </span>
                  <button
                    onClick={handlePublishComment}
                    disabled={publishing || !commentText.trim() || !!replyTo}
                    className="px-4 py-1.5 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {publishing ? "Publishing..." : "Post Comment"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="border border-dashed rounded-lg p-4 mt-4 text-center text-gray-400">
                <p className="text-sm">
                  {hasWallet()
                    ? "Connect your wallet to leave a comment"
                    : "Install MetaMask to leave a comment"}
                </p>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="text-center text-xs text-gray-400 mt-6">
        Published {new Date(appchain.created_at).toLocaleDateString()}
      </div>
    </div>
  );
}

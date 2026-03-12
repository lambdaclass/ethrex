"use client";

import { useState, useEffect, useCallback } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { storeApi } from "@/lib/api";
import {
  type Review,
  type Comment,
  type WalletSession,
  getAppchainReviews,
  getAppchainComments,
  getReactionCounts,
  publishReview,
  publishComment,
  publishReaction,
  connectWallet,
  disconnectWallet,
  hasWallet,
  shortenAddress,
} from "@/lib/nostr";

interface AppchainDetail {
  id: string;
  name: string;
  description: string | null;
  chain_id: number | null;
  rpc_url: string | null;
  status: string;
  phase: string;
  bridge_address: string | null;
  proposer_address: string | null;
  explorer_url: string | null;
  dashboard_url: string | null;
  screenshots: string[];
  social_links: Record<string, string>;
  l1_chain_id: number | null;
  network_mode: string | null;
  program_name: string;
  program_slug: string;
  category: string;
  owner_name: string;
  owner_picture: string | null;
  created_at: number;
}

interface LiveStatus {
  blockNumber: number | null;
  batchNumber: number | null;
  gasPrice: string | null;
  online: boolean;
}

const L1_NAMES: Record<number, string> = {
  1: "Ethereum Mainnet",
  11155111: "Sepolia",
  17000: "Holesky",
};

function ipfsToHttp(uri: string): string {
  if (uri.startsWith("ipfs://")) {
    return `https://gateway.pinata.cloud/ipfs/${uri.replace("ipfs://", "")}`;
  }
  return uri;
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
  const [socialTab, setSocialTab] = useState<"reviews" | "comments">("reviews");
  const [reviewRating, setReviewRating] = useState(5);
  const [reviewText, setReviewText] = useState("");
  const [commentText, setCommentText] = useState("");
  const [publishing, setPublishing] = useState(false);
  const [socialError, setSocialError] = useState<string | null>(null);
  const [likedIds, setLikedIds] = useState<Set<string>>(new Set());

  // Wallet session
  const [walletSession, setWalletSession] = useState<WalletSession | null>(null);
  const [walletConnecting, setWalletConnecting] = useState(false);

  // No session restore — wallet session is in React state only (security).
  // User re-signs on each page load.

  useEffect(() => {
    async function load() {
      try {
        const data = await storeApi.appchain(id);
        setAppchain(data);
      } catch {
        setError("Appchain not found");
      } finally {
        setLoading(false);
      }
    }
    load();
  }, [id]);

  const fetchLiveStatus = useCallback(async () => {
    if (!id) return;
    try {
      const [block, batch, gas] = await Promise.all([
        storeApi.appchainRpc(id, "eth_blockNumber"),
        storeApi.appchainRpc(id, "ethrex_batchNumber"),
        storeApi.appchainRpc(id, "eth_gasPrice"),
      ]);
      setLiveStatus({
        blockNumber: block ? parseInt(block, 16) : null,
        batchNumber: batch ? parseInt(batch, 16) : null,
        gasPrice: gas ? (parseInt(gas, 16) / 1e9).toFixed(4) : null,
        online: block !== null,
      });
    } catch {
      setLiveStatus({ blockNumber: null, batchNumber: null, gasPrice: null, online: false });
    }
  }, [id]);

  useEffect(() => {
    if (!appchain) return;
    fetchLiveStatus();
    const interval = setInterval(fetchLiveStatus, 30000);
    return () => clearInterval(interval);
  }, [appchain, fetchLiveStatus]);

  // Fetch social data (reviews + comments from Nostr relay)
  const fetchSocial = useCallback(async () => {
    if (!appchain?.chain_id) return;
    const chainId = appchain.chain_id.toString();
    try {
      const [r, c] = await Promise.all([
        getAppchainReviews(chainId),
        getAppchainComments(chainId),
      ]);
      setReviews(r);
      setComments(c);
      const allIds = [...r.map((x) => x.id), ...c.map((x) => x.id)];
      if (allIds.length > 0) {
        const counts = await getReactionCounts(allIds);
        setReactionCounts(counts);
      }
    } catch (err) {
      console.warn("[social] Failed to fetch from relay:", err);
    }
  }, [appchain?.chain_id]);

  useEffect(() => {
    fetchSocial();
  }, [fetchSocial]);

  const handleConnectWallet = async () => {
    setWalletConnecting(true);
    setSocialError(null);
    try {
      const session = await connectWallet();
      setWalletSession(session);
    } catch (err) {
      setSocialError(err instanceof Error ? err.message : "Failed to connect wallet");
    } finally {
      setWalletConnecting(false);
    }
  };

  const handleDisconnect = () => {
    disconnectWallet();
    setWalletSession(null);
  };

  const handlePublishReview = async () => {
    if (!reviewText.trim() || !appchain?.chain_id || !walletSession) return;
    setPublishing(true);
    setSocialError(null);
    try {
      await publishReview(walletSession, appchain.chain_id.toString(), reviewRating, reviewText.trim());
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
    if (!commentText.trim() || !appchain?.chain_id || !walletSession) return;
    setPublishing(true);
    setSocialError(null);
    try {
      await publishComment(walletSession, appchain.chain_id.toString(), commentText.trim());
      setCommentText("");
      await fetchSocial();
    } catch (err) {
      setSocialError(err instanceof Error ? err.message : "Failed to publish comment");
    } finally {
      setPublishing(false);
    }
  };

  const handleLike = async (eventId: string) => {
    if (likedIds.has(eventId) || !walletSession) return;
    try {
      await publishReaction(walletSession, eventId);
      setLikedIds((prev) => new Set(prev).add(eventId));
      setReactionCounts((prev) => ({ ...prev, [eventId]: (prev[eventId] || 0) + 1 }));
    } catch (err) {
      console.warn("[social] Failed to publish reaction:", err);
    }
  };

  const copyToClipboard = (text: string, label: string) => {
    navigator.clipboard.writeText(text);
    setCopied(label);
    setTimeout(() => setCopied(null), 2000);
  };

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
        <Link href="/showroom" className="text-blue-600 hover:underline">Back to Showroom</Link>
      </div>
    );
  }

  const l1Name = appchain.l1_chain_id ? (L1_NAMES[appchain.l1_chain_id] || `Chain ${appchain.l1_chain_id}`) : null;
  const contracts = [
    { label: "OnChainProposer", addr: appchain.proposer_address },
    { label: "CommonBridge", addr: appchain.bridge_address },
  ].filter((c) => c.addr);

  const socialEntries = Object.entries(appchain.social_links || {}).filter(([, v]) => v);

  /** Render wallet address or pubkey for a social entry. */
  const renderAuthor = (walletAddress: string | null, pubkey: string) => {
    if (walletAddress) {
      return (
        <span className="text-xs font-mono text-gray-500" title={walletAddress}>
          {shortenAddress(walletAddress)}
        </span>
      );
    }
    return (
      <span className="text-xs font-mono text-gray-400" title={pubkey}>
        {shortenAddress(pubkey)}
      </span>
    );
  };

  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      {/* Back link */}
      <Link href="/showroom" className="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 mb-6">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="15 18 9 12 15 6" />
        </svg>
        Back to Showroom
      </Link>

      {/* Header */}
      <div className="bg-white rounded-xl border p-6 mb-4">
        <div className="flex items-start justify-between">
          <div>
            <h1 className="text-2xl font-bold">{appchain.name}</h1>
            <p className="text-gray-500 mt-1">
              by {appchain.owner_name}
              {l1Name && <span className="ml-2 text-xs px-2 py-0.5 bg-purple-50 text-purple-700 rounded">{l1Name}</span>}
              {appchain.chain_id && <span className="ml-2 text-xs font-mono text-gray-400">Chain ID: {appchain.chain_id}</span>}
            </p>
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
        {appchain.chain_id && appchain.rpc_url && (
          <button
            onClick={async () => {
              try {
                const eth = (window as unknown as { ethereum?: { request: (args: unknown) => Promise<unknown> } }).ethereum;
                if (!eth) return;
                await eth.request({
                  method: "wallet_addEthereumChain",
                  params: [{
                    chainId: `0x${appchain.chain_id!.toString(16)}`,
                    chainName: appchain.name,
                    nativeCurrency: { name: "TON", symbol: "TON", decimals: 18 },
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

      {/* Description */}
      {appchain.description && (
        <div className="bg-white rounded-xl border p-6 mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">About</h2>
          <p className="text-gray-700 whitespace-pre-wrap">{appchain.description}</p>
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

      {/* Services */}
      {(appchain.explorer_url || appchain.dashboard_url || appchain.rpc_url) && (
      <div className="bg-white rounded-xl border p-6 mb-4">
        <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">Services</h2>
        <div className="space-y-3">
          {appchain.explorer_url && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-gray-600">L2 Explorer</span>
              <a href={appchain.explorer_url} target="_blank" rel="noopener noreferrer"
                className="text-sm text-blue-600 hover:underline flex items-center gap-1">
                {appchain.explorer_url.replace(/^https?:\/\//, "").slice(0, 40)}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
              </a>
            </div>
          )}
          {appchain.dashboard_url && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-gray-600">Bridge Dashboard</span>
              <a href={appchain.dashboard_url} target="_blank" rel="noopener noreferrer"
                className="text-sm text-blue-600 hover:underline flex items-center gap-1">
                {appchain.dashboard_url.replace(/^https?:\/\//, "").slice(0, 40)}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
              </a>
            </div>
          )}
          {appchain.rpc_url && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-gray-600">RPC URL</span>
              <button onClick={() => copyToClipboard(appchain.rpc_url!, "rpc")}
                className="text-sm font-mono text-gray-700 hover:text-blue-600 flex items-center gap-1">
                {appchain.rpc_url.length > 40 ? appchain.rpc_url.slice(0, 40) + "..." : appchain.rpc_url}
                <span className="text-xs text-gray-400">{copied === "rpc" ? "Copied!" : ""}</span>
              </button>
            </div>
          )}
        </div>
      </div>
      )}

      {/* L1 Contracts */}
      {contracts.length > 0 && (
        <div className="bg-white rounded-xl border p-6 mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">
            L1 Contracts{l1Name && ` (${l1Name})`}
          </h2>
          <div className="space-y-2">
            {contracts.map((c) => (
              <div key={c.label} className="flex items-center justify-between bg-gray-50 rounded-lg px-4 py-3">
                <div>
                  <div className="text-xs text-gray-500">{c.label}</div>
                  <div className="text-sm font-mono">{shortenAddress(c.addr!)}</div>
                </div>
                <button onClick={() => copyToClipboard(c.addr!, c.label)}
                  className="text-gray-400 hover:text-gray-600 p-1">
                  {copied === c.label ? (
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polyline points="20 6 9 17 4 12"/></svg>
                  ) : (
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
                  )}
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Live Status */}
      <div className="bg-white rounded-xl border p-6 mb-4">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide">Live Status</h2>
          <button onClick={fetchLiveStatus} className="text-xs text-blue-600 hover:underline">Refresh</button>
        </div>
        {liveStatus.online ? (
          <div className="grid grid-cols-3 gap-4">
            <div className="bg-gray-50 rounded-lg p-3 text-center">
              <div className="text-xs text-gray-500 mb-1">Latest Block</div>
              <div className="text-lg font-semibold font-mono">
                {liveStatus.blockNumber?.toLocaleString()}
              </div>
            </div>
            <div className="bg-gray-50 rounded-lg p-3 text-center">
              <div className="text-xs text-gray-500 mb-1">Latest Batch</div>
              <div className="text-lg font-semibold font-mono">
                {liveStatus.batchNumber?.toLocaleString() ?? "—"}
              </div>
            </div>
            <div className="bg-gray-50 rounded-lg p-3 text-center">
              <div className="text-xs text-gray-500 mb-1">Gas Price</div>
              <div className="text-lg font-semibold font-mono">
                {liveStatus.gasPrice ? `${liveStatus.gasPrice} Gwei` : "—"}
              </div>
            </div>
          </div>
        ) : (
          <div className="text-center py-6 text-gray-400">
            <p className="text-sm">Node unreachable — cannot fetch live status</p>
          </div>
        )}
      </div>

      {/* Social Links */}
      {socialEntries.length > 0 && (
        <div className="bg-white rounded-xl border p-6 mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide mb-3">Links</h2>
          <div className="flex gap-3 flex-wrap">
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

      {/* Community (Nostr Social) */}
      <div className="bg-white rounded-xl border p-6 mb-4">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-sm font-semibold text-gray-500 uppercase tracking-wide">Community</h2>
          {/* Wallet Connection */}
          {walletSession ? (
            <div className="flex items-center gap-2">
              <span className="text-xs font-mono text-gray-600 bg-gray-100 px-2 py-1 rounded">
                {shortenAddress(walletSession.address)}
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

        {/* Reviews Tab */}
        {socialTab === "reviews" && (
          <div>
            {/* Write Review — requires wallet */}
            {walletSession ? (
              <div className="border rounded-lg p-4 mb-4 bg-gray-50">
                <div className="flex items-center gap-2 mb-2">
                  <span className="text-sm text-gray-600">Rating:</span>
                  <div className="flex gap-0.5">
                    {[1, 2, 3, 4, 5].map((star) => (
                      <button key={star} onClick={() => setReviewRating(star)} className="p-0.5">
                        <svg width="20" height="20" viewBox="0 0 24 24"
                          fill={star <= reviewRating ? "#f59e0b" : "none"}
                          stroke={star <= reviewRating ? "#f59e0b" : "#d1d5db"}
                          strokeWidth="2">
                          <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                        </svg>
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
                    Posting as {shortenAddress(walletSession.address)}
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
                        {renderAuthor(review.walletAddress, review.pubkey)}
                        <div className="flex gap-0.5">
                          {[1, 2, 3, 4, 5].map((star) => (
                            <svg key={star} width="14" height="14" viewBox="0 0 24 24"
                              fill={star <= review.rating ? "#f59e0b" : "none"}
                              stroke={star <= review.rating ? "#f59e0b" : "#d1d5db"}
                              strokeWidth="2">
                              <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                            </svg>
                          ))}
                        </div>
                      </div>
                      <span className="text-xs text-gray-400">
                        {new Date(review.createdAt * 1000).toLocaleDateString()}
                      </span>
                    </div>
                    <p className="text-sm text-gray-700 whitespace-pre-wrap">{review.content}</p>
                    <div className="mt-2 flex items-center gap-1">
                      <button
                        onClick={() => handleLike(review.id)}
                        disabled={!walletSession}
                        className="flex items-center gap-1 text-xs text-gray-400 hover:text-red-500 disabled:hover:text-gray-400 disabled:cursor-not-allowed transition-colors"
                      >
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                          <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" />
                        </svg>
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
            {/* Write Comment — requires wallet */}
            {walletSession ? (
              <div className="border rounded-lg p-4 mb-4 bg-gray-50">
                <textarea
                  value={commentText}
                  onChange={(e) => setCommentText(e.target.value)}
                  placeholder="Leave a comment..."
                  className="w-full border rounded-lg px-3 py-2 text-sm resize-none h-16 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  maxLength={500}
                />
                <div className="flex items-center justify-between mt-2">
                  <span className="text-xs text-gray-400">
                    Posting as {shortenAddress(walletSession.address)}
                  </span>
                  <button
                    onClick={handlePublishComment}
                    disabled={publishing || !commentText.trim()}
                    className="px-4 py-1.5 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {publishing ? "Publishing..." : "Post Comment"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="border border-dashed rounded-lg p-6 mb-4 text-center text-gray-400">
                <p className="text-sm">
                  {hasWallet()
                    ? "Connect your wallet to leave a comment"
                    : "Install MetaMask to leave a comment"}
                </p>
              </div>
            )}

            {/* Comment List */}
            {comments.length === 0 ? (
              <p className="text-center text-gray-400 text-sm py-6">No comments yet. Start the conversation!</p>
            ) : (
              <div className="space-y-3">
                {comments.map((comment) => (
                  <div key={comment.id} className="border rounded-lg p-4">
                    <div className="flex items-center justify-between mb-2">
                      {renderAuthor(comment.walletAddress, comment.pubkey)}
                      <span className="text-xs text-gray-400">
                        {new Date(comment.createdAt * 1000).toLocaleDateString()}
                      </span>
                    </div>
                    <p className="text-sm text-gray-700 whitespace-pre-wrap">{comment.content}</p>
                    <div className="mt-2 flex items-center gap-1">
                      <button
                        onClick={() => handleLike(comment.id)}
                        disabled={!walletSession}
                        className="flex items-center gap-1 text-xs text-gray-400 hover:text-red-500 disabled:hover:text-gray-400 disabled:cursor-not-allowed transition-colors"
                      >
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                          <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" />
                        </svg>
                        {reactionCounts[comment.id] || 0}
                      </button>
                    </div>
                  </div>
                ))}
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

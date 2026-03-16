"use client";

import { useState, useEffect, useCallback, useMemo } from "react";
import Link from "next/link";
import { storeApi, bookmarkApi, authApi } from "@/lib/api";
import { L1_NAMES, STACK_LABELS } from "@/lib/constants";
import { type Appchain, getAppchainDisplayName, getAppchainChainId } from "@/lib/types";

type Filter = "all" | "bookmarked" | "top-rated" | "newest" | "most-reviewed";

export default function ShowroomPage() {
  const [appchains, setAppchains] = useState<Appchain[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [onlineMap, setOnlineMap] = useState<Record<string, boolean>>({});
  const [filter, setFilter] = useState<Filter>("all");
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [bookmarkedIds, setBookmarkedIds] = useState<Set<string>>(new Set());
  const [isLoggedIn, setIsLoggedIn] = useState(false);
  const [stackFilter, setStackFilter] = useState<string>("");
  const [l1Filter, setL1Filter] = useState<string>("");

  const loadAppchains = useCallback(async (searchTerm?: string) => {
    setLoading(true);
    try {
      const params: { search?: string; stack_type?: string; l1_chain_id?: string } = {};
      if (searchTerm) params.search = searchTerm;
      if (stackFilter) params.stack_type = stackFilter;
      if (l1Filter) params.l1_chain_id = l1Filter;
      const data = await storeApi.appchains(Object.keys(params).length > 0 ? params : undefined);
      setAppchains(data);
    } catch {
      setAppchains([]);
    } finally {
      setLoading(false);
    }
  }, [stackFilter, l1Filter]);

  // Check login status and load bookmarks
  useEffect(() => {
    (async () => {
      try {
        await authApi.me();
        setIsLoggedIn(true);
        const ids = await bookmarkApi.list();
        setBookmarkedIds(new Set(ids));
      } catch {
        setIsLoggedIn(false);
      }
    })();
  }, []);

  useEffect(() => {
    loadAppchains(search || undefined);
  }, [loadAppchains, search]);

  // Check live status for each appchain (fire and forget, no blocking)
  useEffect(() => {
    if (appchains.length === 0) return;
    const checkAll = async () => {
      const results: Record<string, boolean> = {};
      await Promise.allSettled(
        appchains.map(async (chain) => {
          try {
            const block = await storeApi.appchainRpc(chain.id, "eth_blockNumber");
            results[chain.id] = block !== null;
          } catch {
            results[chain.id] = false;
          }
        })
      );
      setOnlineMap(results);
    };
    checkAll();
  }, [appchains]);

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    loadAppchains(search);
  };

  const handleBookmark = async (e: React.MouseEvent, chainId: string) => {
    e.preventDefault(); // Prevent Link navigation
    e.stopPropagation();
    if (!isLoggedIn) return;
    try {
      const result = await bookmarkApi.toggle(chainId);
      setBookmarkedIds((prev) => {
        const next = new Set(prev);
        if (result.bookmarked) next.add(chainId);
        else next.delete(chainId);
        return next;
      });
    } catch {
      // ignore
    }
  };

  // Collect all unique hashtags from appchains
  const allTags = useMemo(() =>
    Array.from(new Set(appchains.flatMap((c) => c.hashtags || []))).sort(),
    [appchains]
  );

  // Apply filter + hashtag
  const filtered = useMemo(() => {
    let list = [...appchains];

    if (selectedTag) {
      list = list.filter((c) => c.hashtags?.includes(selectedTag));
    }

    switch (filter) {
      case "bookmarked":
        list = list.filter((c) => bookmarkedIds.has(c.id));
        break;
      case "top-rated":
        list = list.slice().sort((a, b) => (b.avg_rating ?? 0) - (a.avg_rating ?? 0));
        break;
      case "newest":
        list = list.slice().sort((a, b) => b.created_at - a.created_at);
        break;
      case "most-reviewed":
        list = list.slice().sort((a, b) => b.review_count - a.review_count);
        break;
    }
    return list;
  }, [appchains, filter, selectedTag, bookmarkedIds]);

  const filters: { key: Filter; label: string; loginRequired?: boolean }[] = [
    { key: "all", label: "All" },
    { key: "bookmarked", label: "Bookmarked", loginRequired: true },
    { key: "top-rated", label: "Top Rated" },
    { key: "newest", label: "Newest" },
    { key: "most-reviewed", label: "Most Reviewed" },
  ];

  return (
    <div className="max-w-6xl mx-auto px-4 py-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-3xl font-bold">Explore Appchains</h1>
      </div>

      {/* Search bar (full width, same as Store) */}
      <form onSubmit={handleSearch} className="flex gap-4 mb-6">
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search appchains..."
          className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        />
        <button
          type="submit"
          className="px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700"
        >
          Search
        </button>
      </form>

      {/* Stack type & L1 network filters */}
      <div className="flex gap-3 mb-4">
        <select
          value={stackFilter}
          onChange={(e) => setStackFilter(e.target.value)}
          className="px-3 py-1.5 border border-gray-300 rounded-lg text-sm focus:ring-2 focus:ring-blue-500"
        >
          <option value="">All Stacks</option>
          {Object.entries(STACK_LABELS).map(([key, label]) => (
            <option key={key} value={key}>{label}</option>
          ))}
        </select>
        <select
          value={l1Filter}
          onChange={(e) => setL1Filter(e.target.value)}
          className="px-3 py-1.5 border border-gray-300 rounded-lg text-sm focus:ring-2 focus:ring-blue-500"
        >
          <option value="">All L1 Networks</option>
          {Object.entries(L1_NAMES).map(([id, name]) => (
            <option key={id} value={id}>{name}</option>
          ))}
        </select>
      </div>

      {/* Sort filters + hashtags */}
      <div className="flex items-center gap-4 mb-6 flex-wrap">
        <div className="flex gap-1.5 flex-wrap">
          {filters.map((f) => {
            if (f.loginRequired && !isLoggedIn) return null;
            return (
              <button
                key={f.key}
                onClick={() => setFilter(f.key)}
                className={`px-3 py-1 rounded-full text-xs font-medium transition-colors ${
                  filter === f.key
                    ? "bg-blue-600 text-white"
                    : "bg-gray-100 text-gray-600 hover:bg-gray-200"
                }`}
              >
                {f.key === "bookmarked" && (
                  <span className="mr-1">&#9733;</span>
                )}
                {f.label}
              </button>
            );
          })}
        </div>
        {allTags.length > 0 && (
          <>
            <span className="text-gray-300">|</span>
            <div className="flex gap-1.5 flex-wrap">
              {allTags.map((tag) => (
                <button
                  key={tag}
                  onClick={() => setSelectedTag(selectedTag === tag ? null : tag)}
                  className={`px-2.5 py-1 rounded-full text-xs font-medium transition-colors ${
                    selectedTag === tag
                      ? "bg-purple-600 text-white"
                      : "bg-purple-50 text-purple-700 hover:bg-purple-100"
                  }`}
                >
                  #{tag}
                </button>
              ))}
            </div>
          </>
        )}
      </div>

      {loading ? (
        <div className="flex justify-center py-16">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
        </div>
      ) : filtered.length === 0 ? (
        <div className="text-center py-16">
          <div className="w-16 h-16 mx-auto mb-4 bg-gray-100 rounded-2xl flex items-center justify-center">
            <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" className="text-gray-400">
              <circle cx="12" cy="12" r="10"/>
              <path d="M8 12h8M12 8v8"/>
            </svg>
          </div>
          <h2 className="text-lg font-semibold text-gray-700 mb-2">
            {filter === "bookmarked" ? "No Bookmarked Appchains" : "No Public Appchains Yet"}
          </h2>
          <p className="text-gray-500">
            {filter === "bookmarked"
              ? "Bookmark appchains you like to find them easily later."
              : "Be the first to publish your appchain! Use the Desktop App to create and publish."}
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {filtered.map((chain) => {
            const online = onlineMap[chain.id];
            const l1Name = chain.l1_chain_id ? (L1_NAMES[chain.l1_chain_id] || `Chain ${chain.l1_chain_id}`) : null;
            const stackLabel = chain.stack_type ? STACK_LABELS[chain.stack_type] || chain.stack_type : null;
            const displayName = chain.operator_name || chain.owner_name;
            const chainId = getAppchainChainId(chain);
            const isBookmarked = bookmarkedIds.has(chain.id);
            return (
              <Link href={`/explore/${chain.id}`} key={chain.id}>
                <div className="bg-white rounded-xl border p-6 hover:shadow-md transition-shadow cursor-pointer h-full flex flex-col">
                  <div className="flex items-start justify-between mb-3">
                    <div>
                      <h3 className="font-semibold text-lg">{chain.name}</h3>
                      <p className="text-sm text-gray-500">
                        by {displayName}
                        {l1Name && <span className="ml-1.5 text-xs text-purple-600">{l1Name}</span>}
                      </p>
                    </div>
                    <div className="flex items-center gap-1.5">
                      {isLoggedIn && (
                        <button
                          onClick={(e) => handleBookmark(e, chain.id)}
                          className="p-1 rounded hover:bg-gray-100 transition-colors"
                          title={isBookmarked ? "Remove bookmark" : "Bookmark"}
                        >
                          <svg width="16" height="16" viewBox="0 0 24 24"
                            fill={isBookmarked ? "#2563eb" : "none"}
                            stroke={isBookmarked ? "#2563eb" : "#9ca3af"}
                            strokeWidth="2"
                          >
                            <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
                          </svg>
                        </button>
                      )}
                      {chain.network_mode && (
                        <span className="px-2 py-0.5 bg-purple-50 text-purple-700 rounded text-xs font-medium">
                          {chain.network_mode}
                        </span>
                      )}
                      {online === true ? (
                        <span className="flex items-center gap-1 px-2 py-0.5 bg-green-50 text-green-700 rounded text-xs font-medium">
                          <span className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse" />
                          Online
                        </span>
                      ) : online === false ? (
                        <span className="px-2 py-0.5 bg-gray-100 text-gray-500 rounded text-xs font-medium">
                          Offline
                        </span>
                      ) : (
                        <span className="px-2 py-0.5 bg-green-100 text-green-700 rounded text-xs font-medium">
                          Active
                        </span>
                      )}
                    </div>
                  </div>

                  {chain.description && (
                    <p className="text-sm text-gray-600 mb-3 line-clamp-2">{chain.description}</p>
                  )}

                  {chain.hashtags?.length > 0 && (
                    <div className="flex flex-wrap gap-1 mb-3">
                      {chain.hashtags.slice(0, 4).map((tag) => (
                        <span key={tag} className="px-2 py-0.5 bg-purple-50 text-purple-600 rounded text-xs">
                          #{tag}
                        </span>
                      ))}
                    </div>
                  )}

                  <div className="space-y-2 mb-4">
                    {stackLabel ? (
                      <div className="flex items-center gap-2 text-sm">
                        <span className="text-gray-500">Stack:</span>
                        <span className="px-2 py-0.5 bg-indigo-50 text-indigo-700 rounded text-xs">
                          {stackLabel}
                        </span>
                      </div>
                    ) : chain.program_name ? (
                      <div className="flex items-center gap-2 text-sm">
                        <span className="text-gray-500">Program:</span>
                        <span className="px-2 py-0.5 bg-blue-50 text-blue-700 rounded text-xs">
                          {chain.program_name}
                        </span>
                      </div>
                    ) : null}
                    {chainId && (
                      <div className="flex items-center gap-2 text-sm">
                        <span className="text-gray-500">Chain ID:</span>
                        <span className="font-mono">{chainId}</span>
                      </div>
                    )}
                    {chain.native_token_symbol && chain.native_token_symbol !== "ETH" && (
                      <div className="flex items-center gap-2 text-sm">
                        <span className="text-gray-500">Token:</span>
                        <span className="font-mono">{chain.native_token_symbol}</span>
                      </div>
                    )}
                  </div>

                  {/* Social stats — always show for consistent card height */}
                  <div className="flex items-center gap-3 text-sm text-gray-500 mb-3">
                    {chain.avg_rating !== null ? (
                      <span className="flex items-center gap-1">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="2">
                          <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                        </svg>
                        {chain.avg_rating.toFixed(1)}
                      </span>
                    ) : (
                      <span className="flex items-center gap-1">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#d1d5db" strokeWidth="2">
                          <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                        </svg>
                        <span className="text-gray-400">No reviews yet</span>
                      </span>
                    )}
                    {chain.review_count > 0 && (
                      <span>{chain.review_count} review{chain.review_count !== 1 ? "s" : ""}</span>
                    )}
                    {chain.comment_count > 0 && (
                      <span>{chain.comment_count} comment{chain.comment_count !== 1 ? "s" : ""}</span>
                    )}
                  </div>

                  <div className="flex items-center justify-between pt-3 border-t mt-auto">
                    <span className="text-xs text-gray-400">
                      {new Date(chain.created_at).toLocaleDateString()}
                    </span>
                    <span className="text-xs px-2 py-0.5 bg-gray-100 rounded">{chain.category}</span>
                  </div>
                </div>
              </Link>
            );
          })}
        </div>
      )}
    </div>
  );
}

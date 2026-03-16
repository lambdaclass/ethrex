"use client";

import { useState, useEffect, useCallback, useMemo } from "react";
import Link from "next/link";
import { storeApi, bookmarkApi, authApi } from "@/lib/api";
import { L1_NAMES, STACK_LABELS } from "@/lib/constants";
import { type Appchain, getAppchainChainId } from "@/lib/types";

type Filter = "all" | "bookmarked";
type SortKey = "name" | "stack" | "chainid" | "rating" | "reviews" | "comments" | "created";
type SortDir = "asc" | "desc";

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
  const [syncing, setSyncing] = useState(false);
  const [syncMsg, setSyncMsg] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("created");
  const [sortDir, setSortDir] = useState<SortDir>("desc");

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
    e.preventDefault();
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

  const allTags = useMemo(() =>
    Array.from(new Set(appchains.flatMap((c) => c.hashtags || []))).sort(),
    [appchains]
  );

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDir(sortDir === "asc" ? "desc" : "asc");
    } else {
      setSortKey(key);
      setSortDir(key === "name" ? "asc" : "desc");
    }
  };

  const filtered = useMemo(() => {
    let list = [...appchains];

    if (filter === "bookmarked") {
      list = list.filter((c) => bookmarkedIds.has(c.id));
    }
    if (selectedTag) {
      list = list.filter((c) => c.hashtags?.includes(selectedTag));
    }

    const dir = sortDir === "asc" ? 1 : -1;
    list.sort((a, b) => {
      switch (sortKey) {
        case "name":
          return dir * a.name.localeCompare(b.name);
        case "stack": {
          const sa = a.stack_type ? (STACK_LABELS[a.stack_type] || a.stack_type) : a.program_name || "";
          const sb = b.stack_type ? (STACK_LABELS[b.stack_type] || b.stack_type) : b.program_name || "";
          return dir * sa.localeCompare(sb);
        }
        case "chainid":
          return dir * ((getAppchainChainId(a) ?? 0) - (getAppchainChainId(b) ?? 0));
        case "rating":
          return dir * ((a.avg_rating ?? 0) - (b.avg_rating ?? 0));
        case "reviews":
          return dir * (a.review_count - b.review_count);
        case "comments":
          return dir * (a.comment_count - b.comment_count);
        case "created":
          return dir * (a.created_at - b.created_at);
        default:
          return 0;
      }
    });

    return list;
  }, [appchains, filter, selectedTag, bookmarkedIds, sortKey, sortDir]);

  const SortHeader = ({ label, sortKeyVal, className }: { label: string; sortKeyVal: SortKey; className?: string }) => (
    <th
      className={`px-4 py-3 text-left text-xs font-semibold text-gray-500 uppercase tracking-wider cursor-pointer select-none hover:text-gray-700 transition-colors ${className || ""}`}
      onClick={() => toggleSort(sortKeyVal)}
    >
      <span className="inline-flex items-center gap-1">
        {label}
        {sortKey === sortKeyVal ? (
          <span className="text-blue-600">{sortDir === "asc" ? "\u2191" : "\u2193"}</span>
        ) : (
          <span className="text-gray-300">\u2195</span>
        )}
      </span>
    </th>
  );

  return (
    <div className="max-w-7xl mx-auto px-4 py-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-3xl font-bold">Explore Appchains</h1>
        <button
          onClick={async () => {
            setSyncing(true);
            setSyncMsg(null);
            try {
              const res = await fetch("/api/cron/metadata-sync");
              const data = await res.json();
              if (data.ok) {
                setSyncMsg(`Synced: ${data.synced}, Deleted: ${data.deleted}, Errors: ${data.errors} (${data.elapsed_ms}ms)`);
                loadAppchains(search || undefined);
              } else if (data.cooldown) {
                setSyncMsg(`Sync available in ${data.remain_sec}s (5-min cooldown)`);
              } else {
                setSyncMsg(`Error: ${data.error}`);
              }
            } catch (err) {
              setSyncMsg(`Failed: ${err instanceof Error ? err.message : "Unknown error"}`);
            } finally {
              setSyncing(false);
            }
          }}
          disabled={syncing}
          className="px-4 py-2 bg-indigo-600 text-white rounded-lg text-sm hover:bg-indigo-700 disabled:opacity-50 transition-colors"
        >
          {syncing ? "Syncing..." : "Sync Metadata"}
        </button>
      </div>

      {syncMsg && (
        <div className={`mb-4 px-4 py-3 rounded-lg text-sm ${syncMsg.startsWith("Synced") ? "bg-green-50 text-green-800" : "bg-yellow-50 text-yellow-800"}`}>
          {syncMsg}
        </div>
      )}

      {/* Search + filters */}
      <form onSubmit={handleSearch} className="flex gap-2 mb-4 items-center">
        <select
          value={stackFilter}
          onChange={(e) => setStackFilter(e.target.value)}
          className="px-3 py-2 border border-gray-300 rounded-lg text-sm focus:ring-2 focus:ring-blue-500"
        >
          <option value="">All Stacks</option>
          {Object.entries(STACK_LABELS).map(([key, label]) => (
            <option key={key} value={key}>{label}</option>
          ))}
        </select>
        <select
          value={l1Filter}
          onChange={(e) => setL1Filter(e.target.value)}
          className="px-3 py-2 border border-gray-300 rounded-lg text-sm focus:ring-2 focus:ring-blue-500"
        >
          <option value="">All L1 Networks</option>
          {Object.entries(L1_NAMES).map(([id, name]) => (
            <option key={id} value={id}>{name}</option>
          ))}
        </select>
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search appchains..."
          className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        />
        <button
          type="submit"
          className="px-5 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700"
        >
          Search
        </button>
      </form>

      {/* Filter tabs + hashtags */}
      <div className="flex items-center gap-4 mb-6 flex-wrap">
        <div className="flex gap-1.5 flex-wrap">
          <button
            onClick={() => setFilter("all")}
            className={`px-3 py-1 rounded-full text-xs font-medium transition-colors ${
              filter === "all" ? "bg-blue-600 text-white" : "bg-gray-100 text-gray-600 hover:bg-gray-200"
            }`}
          >
            All
          </button>
          {isLoggedIn && (
            <button
              onClick={() => setFilter("bookmarked")}
              className={`px-3 py-1 rounded-full text-xs font-medium transition-colors ${
                filter === "bookmarked" ? "bg-blue-600 text-white" : "bg-gray-100 text-gray-600 hover:bg-gray-200"
              }`}
            >
              &#9733; Bookmarked
            </button>
          )}
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
        <div className="bg-white rounded-xl border overflow-hidden">
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead className="bg-gray-50 border-b">
                <tr>
                  <th className="w-8 px-4 py-3" />
                  <SortHeader label="Name" sortKeyVal="name" />
                  <SortHeader label="Stack" sortKeyVal="stack" />
                  <SortHeader label="Chain ID" sortKeyVal="chainid" />
                  <th className="px-4 py-3 text-left text-xs font-semibold text-gray-500 uppercase tracking-wider">Status</th>
                  <SortHeader label="Rating" sortKeyVal="rating" className="text-center" />
                  <SortHeader label="Reviews" sortKeyVal="reviews" className="text-center" />
                  <SortHeader label="Comments" sortKeyVal="comments" className="text-center" />
                  <SortHeader label="Created" sortKeyVal="created" />
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100">
                {filtered.map((chain) => {
                  const online = onlineMap[chain.id];
                  const l1Id = chain.l1_chain_id ? Number(chain.l1_chain_id) : null;
                  const l1Name = l1Id ? (L1_NAMES[l1Id] || `Chain ${l1Id}`) : null;
                  const stackLabel = chain.stack_type ? STACK_LABELS[chain.stack_type] || chain.stack_type : chain.program_name || null;
                  const rawName = chain.operator_name || chain.owner_name || "";
                  const displayName = rawName.startsWith("0x") && rawName.length > 12
                    ? `${rawName.slice(0, 6)}...${rawName.slice(-4)}`
                    : rawName;
                  const chainId = getAppchainChainId(chain);
                  const isBookmarked = bookmarkedIds.has(chain.id);

                  return (
                    <tr
                      key={chain.id}
                      className="hover:bg-gray-50 transition-colors cursor-pointer group"
                      onClick={() => window.location.href = `/explore/${chain.id}`}
                    >
                      {/* Bookmark */}
                      <td className="px-4 py-3">
                        {isLoggedIn && (
                          <button
                            onClick={(e) => handleBookmark(e, chain.id)}
                            className="p-1 rounded hover:bg-gray-100 transition-colors"
                            title={isBookmarked ? "Remove bookmark" : "Bookmark"}
                          >
                            <svg width="14" height="14" viewBox="0 0 24 24"
                              fill={isBookmarked ? "#2563eb" : "none"}
                              stroke={isBookmarked ? "#2563eb" : "#d1d5db"}
                              strokeWidth="2"
                            >
                              <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
                            </svg>
                          </button>
                        )}
                      </td>

                      {/* Name + operator + L1 + tags */}
                      <td className="px-4 py-3 min-w-[200px]">
                        <Link href={`/explore/${chain.id}`} className="block">
                          <div className="font-medium text-gray-900 group-hover:text-blue-600 transition-colors">
                            {chain.name}
                          </div>
                          <div className="text-xs text-gray-500 mt-0.5">
                            {displayName && <span>by {displayName}</span>}
                            {l1Name && <span className="ml-1.5 text-purple-600">{l1Name}</span>}
                          </div>
                          {chain.hashtags?.length > 0 && (
                            <div className="flex gap-1 mt-1 flex-wrap">
                              {chain.hashtags.slice(0, 3).map((tag) => (
                                <span key={tag} className="px-1.5 py-0.5 bg-purple-50 text-purple-600 rounded text-[10px]">
                                  #{tag}
                                </span>
                              ))}
                            </div>
                          )}
                        </Link>
                      </td>

                      {/* Stack */}
                      <td className="px-4 py-3">
                        {stackLabel && (
                          <span className="px-2 py-0.5 bg-indigo-50 text-indigo-700 rounded text-xs whitespace-nowrap">
                            {stackLabel}
                          </span>
                        )}
                      </td>

                      {/* Chain ID */}
                      <td className="px-4 py-3">
                        {chainId && <span className="font-mono text-sm text-gray-700">{chainId}</span>}
                      </td>

                      {/* Status */}
                      <td className="px-4 py-3">
                        <div className="flex items-center gap-1.5">
                          {online === true ? (
                            <span className="inline-flex items-center gap-1 px-2 py-0.5 bg-green-50 text-green-700 rounded text-xs font-medium">
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
                          {chain.network_mode && (
                            <span className="px-1.5 py-0.5 bg-purple-50 text-purple-700 rounded text-[10px] font-medium">
                              {chain.network_mode}
                            </span>
                          )}
                        </div>
                      </td>

                      {/* Rating */}
                      <td className="px-4 py-3 text-center">
                        {chain.avg_rating !== null ? (
                          <span className="inline-flex items-center gap-1 text-sm">
                            <svg width="13" height="13" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="2">
                              <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                            </svg>
                            <span className="font-medium">{chain.avg_rating.toFixed(1)}</span>
                          </span>
                        ) : (
                          <span className="text-gray-300 text-sm">-</span>
                        )}
                      </td>

                      {/* Reviews */}
                      <td className="px-4 py-3 text-center">
                        <span className={`text-sm ${chain.review_count > 0 ? "text-gray-700" : "text-gray-300"}`}>
                          {chain.review_count}
                        </span>
                      </td>

                      {/* Comments */}
                      <td className="px-4 py-3 text-center">
                        <span className={`text-sm ${chain.comment_count > 0 ? "text-gray-700" : "text-gray-300"}`}>
                          {chain.comment_count}
                        </span>
                      </td>

                      {/* Created */}
                      <td className="px-4 py-3">
                        <span className="text-xs text-gray-500 whitespace-nowrap">
                          {new Date(Number(chain.created_at)).toLocaleDateString()}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}

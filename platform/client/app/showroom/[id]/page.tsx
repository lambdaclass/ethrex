"use client";

import { useState, useEffect, useCallback } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { storeApi } from "@/lib/api";

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

function shortenAddress(addr: string): string {
  return `${addr.slice(0, 8)}...${addr.slice(-6)}`;
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
  }, [id]);

  useEffect(() => {
    if (!appchain) return;
    fetchLiveStatus();
    const interval = setInterval(fetchLiveStatus, 30000);
    return () => clearInterval(interval);
  }, [appchain, fetchLiveStatus]);

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

      {/* Footer */}
      <div className="text-center text-xs text-gray-400 mt-6">
        Published {new Date(appchain.created_at).toLocaleDateString()}
      </div>
    </div>
  );
}

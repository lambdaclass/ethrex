"use client";

import { useState, useEffect } from "react";
import { storeApi } from "@/lib/api";

interface Appchain {
  id: string;
  name: string;
  chain_id: number | null;
  rpc_url: string | null;
  status: string;
  phase: string;
  bridge_address: string | null;
  proposer_address: string | null;
  program_name: string;
  program_slug: string;
  category: string;
  owner_name: string;
  created_at: number;
}

export default function ShowroomPage() {
  const [appchains, setAppchains] = useState<Appchain[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");

  useEffect(() => {
    loadAppchains();
  }, []);

  const loadAppchains = async (searchTerm?: string) => {
    setLoading(true);
    try {
      const data = await storeApi.appchains(searchTerm ? { search: searchTerm } : undefined);
      setAppchains(data);
    } catch {
      setAppchains([]);
    } finally {
      setLoading(false);
    }
  };

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    loadAppchains(search);
  };

  return (
    <div className="max-w-6xl mx-auto px-4 py-8">
      <div className="text-center mb-8">
        <h1 className="text-3xl font-bold mb-3">Open Appchain Showroom</h1>
        <p className="text-gray-600 max-w-2xl mx-auto">
          Explore public Layer 2 appchains built by the community.
          Connect to them or get inspired to launch your own.
        </p>
      </div>

      <form onSubmit={handleSearch} className="max-w-md mx-auto mb-8">
        <div className="flex gap-2">
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search appchains..."
            className="flex-1 px-4 py-2.5 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          />
          <button
            type="submit"
            className="px-6 py-2.5 bg-blue-600 text-white rounded-lg hover:bg-blue-700"
          >
            Search
          </button>
        </div>
      </form>

      {loading ? (
        <div className="flex justify-center py-16">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
        </div>
      ) : appchains.length === 0 ? (
        <div className="text-center py-16">
          <div className="w-16 h-16 mx-auto mb-4 bg-gray-100 rounded-2xl flex items-center justify-center">
            <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" className="text-gray-400">
              <circle cx="12" cy="12" r="10"/>
              <path d="M8 12h8M12 8v8"/>
            </svg>
          </div>
          <h2 className="text-lg font-semibold text-gray-700 mb-2">No Public Appchains Yet</h2>
          <p className="text-gray-500">
            Be the first to publish your appchain! Use the Desktop App to create and publish.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {appchains.map((chain) => (
            <div
              key={chain.id}
              className="bg-white rounded-xl border p-6 hover:shadow-md transition-shadow"
            >
              <div className="flex items-start justify-between mb-3">
                <div>
                  <h3 className="font-semibold text-lg">{chain.name}</h3>
                  <p className="text-sm text-gray-500">by {chain.owner_name}</p>
                </div>
                <span className="px-2 py-0.5 bg-green-100 text-green-700 rounded text-xs font-medium">
                  Active
                </span>
              </div>

              <div className="space-y-2 mb-4">
                <div className="flex items-center gap-2 text-sm">
                  <span className="text-gray-500">Program:</span>
                  <span className="px-2 py-0.5 bg-blue-50 text-blue-700 rounded text-xs">
                    {chain.program_name}
                  </span>
                </div>
                {chain.chain_id && (
                  <div className="flex items-center gap-2 text-sm">
                    <span className="text-gray-500">Chain ID:</span>
                    <span className="font-mono">{chain.chain_id}</span>
                  </div>
                )}
                {chain.rpc_url && (
                  <div className="flex items-center gap-2 text-sm">
                    <span className="text-gray-500">RPC:</span>
                    <span className="font-mono text-xs truncate max-w-[200px]">{chain.rpc_url}</span>
                  </div>
                )}
                {chain.bridge_address && (
                  <div className="flex items-center gap-2 text-sm">
                    <span className="text-gray-500">Bridge:</span>
                    <span className="font-mono text-xs truncate max-w-[200px]">{chain.bridge_address}</span>
                  </div>
                )}
              </div>

              <div className="flex items-center justify-between pt-3 border-t">
                <span className="text-xs text-gray-400">
                  {new Date(chain.created_at).toLocaleDateString()}
                </span>
                <span className="text-xs px-2 py-0.5 bg-gray-100 rounded">{chain.category}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

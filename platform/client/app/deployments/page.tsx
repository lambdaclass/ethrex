"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { deploymentsApi } from "@/lib/api";
import { useAuth } from "@/components/auth-provider";
import { Deployment } from "@/lib/types";

export default function DeploymentsPage() {
  const { user } = useAuth();
  const [deployments, setDeployments] = useState<Deployment[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!user) return;
    deploymentsApi
      .list()
      .then(setDeployments)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [user]);

  if (!user) {
    return (
      <div className="max-w-4xl mx-auto px-4 py-16 text-center">
        <h1 className="text-xl font-bold mb-4">Login Required</h1>
        <Link href="/login" className="text-blue-600 hover:underline">
          Login to view your deployments
        </Link>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="min-h-[60vh] flex items-center justify-center">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">My Deployments</h1>
        <Link
          href="/store"
          className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700"
        >
          Browse Programs
        </Link>
      </div>

      {deployments.length === 0 ? (
        <div className="text-center py-16 bg-white rounded-xl border">
          <p className="text-gray-500 mb-4">No deployments yet.</p>
          <Link href="/store" className="text-blue-600 hover:underline">
            Browse the Store to get started
          </Link>
        </div>
      ) : (
        <div className="space-y-4">
          {deployments.map((d) => (
            <div
              key={d.id}
              className="bg-white rounded-xl border p-6 flex items-center justify-between"
            >
              <div>
                <h3 className="font-semibold">{d.name}</h3>
                <p className="text-sm text-gray-500">
                  Program: {d.program_name || d.program_id}
                  {d.category && (
                    <span className="ml-2 px-2 py-0.5 bg-gray-100 rounded text-xs">
                      {d.category}
                    </span>
                  )}
                </p>
                <div className="flex gap-4 mt-1 text-xs text-gray-400">
                  {d.chain_id && <span>Chain ID: {d.chain_id}</span>}
                  <span>Created: {new Date(d.created_at).toLocaleDateString()}</span>
                </div>
              </div>
              <div className="flex items-center gap-3">
                <span
                  className={`px-2 py-1 rounded text-xs font-medium ${
                    d.status === "configured"
                      ? "bg-yellow-100 text-yellow-700"
                      : d.status === "active"
                      ? "bg-green-100 text-green-700"
                      : "bg-gray-100 text-gray-600"
                  }`}
                >
                  {d.status}
                </span>
                <Link
                  href={`/deployments/${d.id}`}
                  className="text-blue-600 hover:underline text-sm"
                >
                  Details
                </Link>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { deploymentsApi } from "@/lib/api";
import { useAuth } from "@/components/auth-provider";
import { Deployment } from "@/lib/types";
import { DeploymentStatusBadge } from "@/components/deployment-status";

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
          Login to view your L2s
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
        <h1 className="text-2xl font-bold">My L2s</h1>
        <Link
          href="/launch"
          className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700"
        >
          Launch New L2
        </Link>
      </div>

      {deployments.length === 0 ? (
        <div className="text-center py-16 bg-white rounded-xl border">
          <p className="text-gray-500 mb-4">No L2s launched yet.</p>
          <Link href="/launch" className="text-blue-600 hover:underline">
            Launch your first L2
          </Link>
        </div>
      ) : (
        <div className="space-y-4">
          {deployments.map((d) => (
            <Link
              key={d.id}
              href={`/deployments/${d.id}`}
              className="block bg-white rounded-xl border p-6 hover:shadow-md transition-shadow"
            >
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="font-semibold">{d.name}</h3>
                  <p className="text-sm text-gray-500">
                    App: {d.program_name || d.program_slug}
                    {d.category && (
                      <span className="ml-2 px-2 py-0.5 bg-gray-100 rounded text-xs">
                        {d.category}
                      </span>
                    )}
                  </p>
                  <div className="flex gap-4 mt-1 text-xs text-gray-400">
                    {d.chain_id && <span>Chain ID: {d.chain_id}</span>}
                    {d.l1_port && <span>L1: :{d.l1_port}</span>}
                    {d.l2_port && <span>L2: :{d.l2_port}</span>}
                    <span>Created: {new Date(d.created_at).toLocaleDateString()}</span>
                  </div>
                </div>
                <DeploymentStatusBadge phase={d.phase} />
              </div>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

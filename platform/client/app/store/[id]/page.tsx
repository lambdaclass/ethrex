"use client";

import { useState, useEffect } from "react";
import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { storeApi, deploymentsApi } from "@/lib/api";
import { Program } from "@/lib/types";
import { useAuth } from "@/components/auth-provider";

export default function ProgramDetailPage() {
  const params = useParams();
  const router = useRouter();
  const { user } = useAuth();
  const [program, setProgram] = useState<Program | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  // "Use This Program" modal state
  const [showModal, setShowModal] = useState(false);
  const [deployName, setDeployName] = useState("");
  const [chainId, setChainId] = useState("");
  const [rpcUrl, setRpcUrl] = useState("");
  const [deploying, setDeploying] = useState(false);
  const [deployError, setDeployError] = useState("");

  useEffect(() => {
    if (!params.id) return;
    storeApi
      .program(params.id as string)
      .then((data) => setProgram(data))
      .catch((err) => setError(err instanceof Error ? err.message : "Failed to load program"))
      .finally(() => setLoading(false));
  }, [params.id]);

  const handleUseProgram = async () => {
    if (!deployName.trim()) {
      setDeployError("Deployment name is required");
      return;
    }
    setDeploying(true);
    setDeployError("");
    try {
      await deploymentsApi.create({
        programId: program!.id,
        name: deployName.trim(),
        chainId: chainId ? parseInt(chainId) : undefined,
        rpcUrl: rpcUrl || undefined,
      });
      router.push("/deployments");
    } catch (err) {
      setDeployError(err instanceof Error ? err.message : "Failed to create deployment");
    } finally {
      setDeploying(false);
    }
  };

  if (loading) {
    return (
      <div className="min-h-[60vh] flex items-center justify-center">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
      </div>
    );
  }

  if (error || !program) {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-xl font-bold text-red-600 mb-4">Program Not Found</h1>
        <p className="text-gray-600 mb-4">{error}</p>
        <Link href="/store" className="text-blue-600 hover:underline">
          Back to Store
        </Link>
      </div>
    );
  }

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <Link href="/store" className="text-blue-600 hover:underline text-sm mb-4 inline-block">
        &larr; Back to Store
      </Link>

      <div className="bg-white rounded-xl shadow-sm border p-8">
        <div className="flex items-start gap-6 mb-6">
          <div className="w-16 h-16 bg-blue-100 rounded-xl flex items-center justify-center text-blue-600 font-bold text-2xl shrink-0">
            {program.name.charAt(0).toUpperCase()}
          </div>
          <div>
            <h1 className="text-2xl font-bold">{program.name}</h1>
            <p className="text-gray-500">{program.program_id}</p>
            <div className="flex items-center gap-3 mt-2">
              <span className="px-2 py-0.5 bg-gray-100 rounded text-sm">{program.category}</span>
              {program.is_official && (
                <span className="px-2 py-0.5 bg-blue-100 text-blue-700 rounded text-sm">
                  Official
                </span>
              )}
              {program.program_type_id !== null && (
                <span className="text-sm text-gray-400">
                  Type ID: {program.program_type_id}
                </span>
              )}
            </div>
          </div>
        </div>

        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Description</h2>
          <p className="text-gray-700 whitespace-pre-wrap">
            {program.description || "No description provided."}
          </p>
        </div>

        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Details</h2>
          <dl className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <dt className="text-gray-500">Status</dt>
              <dd className="font-medium capitalize">{program.status}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Usage Count</dt>
              <dd className="font-medium">{program.use_count}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Batches Proved</dt>
              <dd className="font-medium">{program.batch_count}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Created</dt>
              <dd className="font-medium">
                {new Date(program.created_at).toLocaleDateString()}
              </dd>
            </div>
            {program.elf_hash && (
              <div className="col-span-2">
                <dt className="text-gray-500">ELF Hash</dt>
                <dd className="font-mono text-xs break-all">{program.elf_hash}</dd>
              </div>
            )}
          </dl>
        </div>

        <div className="border-t pt-6">
          {user ? (
            <button
              onClick={() => {
                setDeployName(`${program.name} Deployment`);
                setShowModal(true);
              }}
              className="px-6 py-2.5 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700"
            >
              Use This Program
            </button>
          ) : (
            <Link
              href="/login"
              className="px-6 py-2.5 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 inline-block"
            >
              Login to Use
            </Link>
          )}
        </div>
      </div>

      {/* Use This Program Modal */}
      {showModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-white rounded-xl shadow-xl max-w-md w-full p-6">
            <h2 className="text-xl font-bold mb-4">Configure Deployment</h2>
            <p className="text-sm text-gray-500 mb-4">
              Set up <strong>{program.name}</strong> for your L2 chain.
            </p>

            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Deployment Name *
                </label>
                <input
                  type="text"
                  value={deployName}
                  onChange={(e) => setDeployName(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="My L2 Deployment"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Chain ID (optional)
                </label>
                <input
                  type="number"
                  value={chainId}
                  onChange={(e) => setChainId(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="e.g. 12345"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  L1 RPC URL (optional)
                </label>
                <input
                  type="text"
                  value={rpcUrl}
                  onChange={(e) => setRpcUrl(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="https://..."
                />
              </div>

              {deployError && (
                <p className="text-sm text-red-600">{deployError}</p>
              )}
            </div>

            <div className="flex gap-3 mt-6">
              <button
                onClick={() => setShowModal(false)}
                className="flex-1 px-4 py-2 border border-gray-300 rounded-lg text-gray-700 hover:bg-gray-50"
              >
                Cancel
              </button>
              <button
                onClick={handleUseProgram}
                disabled={deploying}
                className="flex-1 px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50"
              >
                {deploying ? "Creating..." : "Create Deployment"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

"use client";

import { useState, useEffect } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { useAuth } from "@/components/auth-provider";
import { adminApi } from "@/lib/api";
import { Program } from "@/lib/types";

interface CreatorInfo {
  id: string;
  email: string;
  name: string;
  role: string;
}

const statusColors: Record<string, string> = {
  pending: "bg-yellow-100 text-yellow-800",
  active: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
  disabled: "bg-gray-100 text-gray-600",
};

export default function AdminProgramDetailPage() {
  const params = useParams();
  const { user, loading: authLoading } = useAuth();
  const [program, setProgram] = useState<Program | null>(null);
  const [creator, setCreator] = useState<CreatorInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [actionMsg, setActionMsg] = useState("");

  useEffect(() => {
    if (!params.id || !user || user.role !== "admin") return;
    adminApi
      .program(params.id as string)
      .then((data) => {
        setProgram(data.program);
        setCreator(data.creator);
      })
      .catch((err) => setError(err instanceof Error ? err.message : "Failed to load"))
      .finally(() => setLoading(false));
  }, [params.id, user]);

  if (authLoading) return null;

  if (!user || user.role !== "admin") {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Access Denied</h1>
        <p className="text-gray-600">Admin access required.</p>
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

  if (error || !program) {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-xl font-bold text-red-600 mb-4">Program Not Found</h1>
        <p className="text-gray-600 mb-4">{error}</p>
        <Link href="/admin" className="text-blue-600 hover:underline">Back to Admin</Link>
      </div>
    );
  }

  const handleApprove = async () => {
    try {
      const updated = await adminApi.approve(program.id);
      setProgram(updated);
      setActionMsg("Program approved successfully");
    } catch (err) {
      setActionMsg(err instanceof Error ? err.message : "Failed to approve");
    }
  };

  const handleReject = async () => {
    if (!confirm("Reject this program?")) return;
    try {
      const updated = await adminApi.reject(program.id);
      setProgram(updated);
      setActionMsg("Program rejected");
    } catch (err) {
      setActionMsg(err instanceof Error ? err.message : "Failed to reject");
    }
  };

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <Link href="/admin" className="text-blue-600 hover:underline text-sm mb-4 inline-block">
        &larr; Back to Admin
      </Link>

      <div className="bg-white rounded-xl shadow-sm border p-8">
        {/* Header */}
        <div className="flex items-start justify-between mb-6">
          <div className="flex items-start gap-4">
            <div className="w-14 h-14 bg-blue-100 rounded-xl flex items-center justify-center text-blue-600 font-bold text-xl shrink-0">
              {program.name.charAt(0).toUpperCase()}
            </div>
            <div>
              <h1 className="text-2xl font-bold">{program.name}</h1>
              <p className="text-gray-500">{program.program_id}</p>
            </div>
          </div>
          <span className={`px-3 py-1 rounded-full text-sm font-medium ${statusColors[program.status]}`}>
            {program.status}
          </span>
        </div>

        {actionMsg && (
          <div className="p-3 bg-blue-50 text-blue-700 text-sm rounded-lg mb-4">
            {actionMsg}
          </div>
        )}

        {/* Program Details */}
        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Program Info</h2>
          <dl className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <dt className="text-gray-500">Category</dt>
              <dd className="font-medium">{program.category}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Program Type ID</dt>
              <dd className="font-medium">{program.program_type_id ?? "Not assigned"}</dd>
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
              <dt className="text-gray-500">Official</dt>
              <dd className="font-medium">{program.is_official ? "Yes" : "No"}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Created</dt>
              <dd className="font-medium">{new Date(program.created_at).toLocaleString()}</dd>
            </div>
            {program.approved_at && (
              <div>
                <dt className="text-gray-500">Approved</dt>
                <dd className="font-medium">{new Date(program.approved_at).toLocaleString()}</dd>
              </div>
            )}
          </dl>

          {program.description && (
            <div className="mt-4">
              <dt className="text-sm text-gray-500 mb-1">Description</dt>
              <dd className="text-sm text-gray-700 whitespace-pre-wrap">{program.description}</dd>
            </div>
          )}
        </div>

        {/* ELF & VK Info */}
        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Binary & Verification Keys</h2>
          <dl className="space-y-3 text-sm">
            <div>
              <dt className="text-gray-500">ELF Hash</dt>
              <dd className="font-mono text-xs break-all">
                {program.elf_hash || <span className="text-gray-400 italic">Not uploaded</span>}
              </dd>
            </div>
            {program.elf_storage_path && (
              <div>
                <dt className="text-gray-500">ELF Path</dt>
                <dd className="font-mono text-xs break-all text-gray-500">
                  {program.elf_storage_path}
                </dd>
              </div>
            )}
            <div>
              <dt className="text-gray-500">VK SP1</dt>
              <dd className="font-mono text-xs break-all">
                {program.vk_sp1 || <span className="text-gray-400 italic">Not uploaded</span>}
              </dd>
            </div>
            <div>
              <dt className="text-gray-500">VK RISC0</dt>
              <dd className="font-mono text-xs break-all">
                {program.vk_risc0 || <span className="text-gray-400 italic">Not uploaded</span>}
              </dd>
            </div>
          </dl>
        </div>

        {/* Creator Info */}
        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Creator</h2>
          {creator ? (
            <dl className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <dt className="text-gray-500">Name</dt>
                <dd className="font-medium">{creator.name}</dd>
              </div>
              <div>
                <dt className="text-gray-500">Email</dt>
                <dd className="font-medium">{creator.email}</dd>
              </div>
              <div>
                <dt className="text-gray-500">Role</dt>
                <dd className="font-medium capitalize">{creator.role}</dd>
              </div>
            </dl>
          ) : (
            <p className="text-sm text-gray-400">Creator info not available</p>
          )}
        </div>

        {/* Actions */}
        <div className="border-t pt-6 flex gap-3">
          {program.status === "pending" && (
            <>
              <button
                onClick={handleApprove}
                className="px-4 py-2 bg-green-600 text-white rounded-lg text-sm hover:bg-green-700"
              >
                Approve
              </button>
              <button
                onClick={handleReject}
                className="px-4 py-2 border border-red-300 text-red-600 rounded-lg text-sm hover:bg-red-50"
              >
                Reject
              </button>
            </>
          )}
          <Link
            href={`/store/${program.id}`}
            className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
          >
            View in Store
          </Link>
        </div>
      </div>
    </div>
  );
}

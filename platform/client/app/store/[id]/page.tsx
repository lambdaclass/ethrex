"use client";

import { useState, useEffect } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { storeApi } from "@/lib/api";
import { Program } from "@/lib/types";
import { useAuth } from "@/components/auth-provider";

export default function ProgramDetailPage() {
  const params = useParams();
  const { user } = useAuth();
  const [program, setProgram] = useState<Program | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!params.id) return;
    storeApi
      .program(params.id as string)
      .then((data) => setProgram(data))
      .catch((err) => setError(err instanceof Error ? err.message : "Failed to load program"))
      .finally(() => setLoading(false));
  }, [params.id]);

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
            <Link
              href={`/launch?program=${program.id}`}
              className="px-6 py-2.5 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 inline-block"
            >
              Launch L2 with This Program
            </Link>
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
    </div>
  );
}

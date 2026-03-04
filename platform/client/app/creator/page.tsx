"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { useAuth } from "@/components/auth-provider";
import { programsApi } from "@/lib/api";
import { Program } from "@/lib/types";

const statusColors: Record<string, string> = {
  pending: "bg-yellow-100 text-yellow-800",
  active: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
  disabled: "bg-gray-100 text-gray-600",
};

export default function CreatorPage() {
  const { user, loading: authLoading } = useAuth();
  const [programs, setPrograms] = useState<Program[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!user) return;
    programsApi
      .list()
      .then((data) => setPrograms(data))
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [user]);

  if (authLoading) return null;

  if (!user) {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Login Required</h1>
        <p className="text-gray-600 mb-4">You need to login to manage your programs.</p>
        <Link href="/login" className="text-blue-600 hover:underline">
          Go to Login
        </Link>
      </div>
    );
  }

  return (
    <div className="max-w-5xl mx-auto px-4 py-8">
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-3xl font-bold">My Programs</h1>
        <Link
          href="/creator/new"
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 font-medium"
        >
          + New Program
        </Link>
      </div>

      {loading ? (
        <div className="text-center py-16">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto mb-4" />
        </div>
      ) : programs.length === 0 ? (
        <div className="bg-white rounded-xl shadow-sm border p-12 text-center">
          <p className="text-gray-500 text-lg mb-4">You haven't created any programs yet.</p>
          <Link
            href="/creator/new"
            className="px-6 py-2.5 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 inline-block"
          >
            Create Your First Program
          </Link>
        </div>
      ) : (
        <div className="space-y-4">
          {programs.map((program) => (
            <Link
              key={program.id}
              href={`/creator/${program.id}`}
              className="block bg-white rounded-xl shadow-sm border p-6 hover:shadow-md transition-shadow"
            >
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="font-semibold text-lg">{program.name}</h3>
                  <p className="text-sm text-gray-500">{program.program_id}</p>
                </div>
                <div className="flex items-center gap-3">
                  <span className={`px-2 py-0.5 rounded text-xs font-medium ${statusColors[program.status]}`}>
                    {program.status}
                  </span>
                  <span className="text-sm text-gray-500">{program.use_count} uses</span>
                </div>
              </div>
              {program.description && (
                <p className="text-gray-600 text-sm mt-2 line-clamp-1">{program.description}</p>
              )}
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

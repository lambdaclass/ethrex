"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { useAuth } from "@/components/auth-provider";
import { adminApi } from "@/lib/api";
import { Program } from "@/lib/types";

interface Stats {
  users: number;
  programs: number;
  active: number;
  pending: number;
  deployments: number;
}

const statusColors: Record<string, string> = {
  pending: "bg-yellow-100 text-yellow-800",
  active: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
  disabled: "bg-gray-100 text-gray-600",
};

export default function AdminPage() {
  const { user, loading: authLoading } = useAuth();
  const [stats, setStats] = useState<Stats | null>(null);
  const [programs, setPrograms] = useState<Program[]>([]);
  const [filter, setFilter] = useState("pending");
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!user || user.role !== "admin") return;
    adminApi.stats().then(setStats).catch(() => {});
  }, [user]);

  useEffect(() => {
    if (!user || user.role !== "admin") return;
    setLoading(true);
    adminApi
      .programs(filter || undefined)
      .then((data) => setPrograms(data))
      .catch(() => setPrograms([]))
      .finally(() => setLoading(false));
  }, [user, filter]);

  if (authLoading) return null;

  if (!user || user.role !== "admin") {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Access Denied</h1>
        <p className="text-gray-600">Admin access required.</p>
      </div>
    );
  }

  const handleApprove = async (id: string) => {
    try {
      const updated = await adminApi.approve(id);
      setPrograms((prev) => prev.map((p) => (p.id === id ? updated : p)));
      if (stats) setStats({ ...stats, active: stats.active + 1, pending: stats.pending - 1 });
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to approve");
    }
  };

  const handleReject = async (id: string) => {
    if (!confirm("Reject this program?")) return;
    try {
      const updated = await adminApi.reject(id);
      setPrograms((prev) => prev.map((p) => (p.id === id ? updated : p)));
      if (stats) setStats({ ...stats, pending: stats.pending - 1 });
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to reject");
    }
  };

  return (
    <div className="max-w-6xl mx-auto px-4 py-8">
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-3xl font-bold">Admin Dashboard</h1>
        <Link
          href="/admin/users"
          className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
        >
          Manage Users
        </Link>
      </div>

      {stats && (
        <div className="grid grid-cols-2 md:grid-cols-5 gap-4 mb-8">
          <div className="bg-white rounded-xl shadow-sm border p-6">
            <p className="text-2xl font-bold">{stats.users}</p>
            <p className="text-sm text-gray-500">Total Users</p>
          </div>
          <div className="bg-white rounded-xl shadow-sm border p-6">
            <p className="text-2xl font-bold">{stats.programs}</p>
            <p className="text-sm text-gray-500">Total Programs</p>
          </div>
          <div className="bg-white rounded-xl shadow-sm border p-6">
            <p className="text-2xl font-bold text-green-600">{stats.active}</p>
            <p className="text-sm text-gray-500">Active</p>
          </div>
          <div className="bg-white rounded-xl shadow-sm border p-6">
            <p className="text-2xl font-bold text-yellow-600">{stats.pending}</p>
            <p className="text-sm text-gray-500">Pending Review</p>
          </div>
          <div className="bg-white rounded-xl shadow-sm border p-6">
            <p className="text-2xl font-bold text-blue-600">{stats.deployments}</p>
            <p className="text-sm text-gray-500">Deployments</p>
          </div>
        </div>
      )}

      <div className="bg-white rounded-xl shadow-sm border">
        <div className="border-b px-6 py-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold">Programs</h2>
          <div className="flex gap-2">
            {["pending", "active", "rejected", ""].map((s) => (
              <button
                key={s}
                onClick={() => setFilter(s)}
                className={`px-3 py-1 rounded text-sm ${
                  filter === s
                    ? "bg-blue-600 text-white"
                    : "bg-gray-100 text-gray-600 hover:bg-gray-200"
                }`}
              >
                {s || "All"}
              </button>
            ))}
          </div>
        </div>

        {loading ? (
          <div className="p-8 text-center">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto" />
          </div>
        ) : programs.length === 0 ? (
          <div className="p-8 text-center text-gray-500">No programs found</div>
        ) : (
          <div className="divide-y">
            {programs.map((program) => (
              <div key={program.id} className="px-6 py-4 flex items-center justify-between">
                <div>
                  <div className="flex items-center gap-3">
                    <h3 className="font-medium">{program.name}</h3>
                    <span className={`px-2 py-0.5 rounded text-xs font-medium ${statusColors[program.status]}`}>
                      {program.status}
                    </span>
                  </div>
                  <p className="text-sm text-gray-500">
                    {program.program_id} &middot; {program.category} &middot; {program.use_count} uses
                  </p>
                </div>
                <div className="flex gap-2">
                  {program.status === "pending" && (
                    <>
                      <button
                        onClick={() => handleApprove(program.id)}
                        className="px-3 py-1.5 bg-green-600 text-white rounded-lg text-sm hover:bg-green-700"
                      >
                        Approve
                      </button>
                      <button
                        onClick={() => handleReject(program.id)}
                        className="px-3 py-1.5 border border-red-300 text-red-600 rounded-lg text-sm hover:bg-red-50"
                      >
                        Reject
                      </button>
                    </>
                  )}
                  <Link
                    href={`/admin/programs/${program.id}`}
                    className="px-3 py-1.5 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
                  >
                    Details
                  </Link>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

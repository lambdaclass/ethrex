"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { useAuth } from "@/components/auth-provider";
import { adminApi } from "@/lib/api";

interface AdminUser {
  id: string;
  email: string;
  name: string;
  role: string;
  auth_provider: string;
  status: string;
  created_at: number;
}

const roleColors: Record<string, string> = {
  admin: "bg-purple-100 text-purple-800",
  user: "bg-gray-100 text-gray-600",
};

const statusColors: Record<string, string> = {
  active: "bg-green-100 text-green-800",
  suspended: "bg-red-100 text-red-800",
};

export default function AdminUsersPage() {
  const { user, loading: authLoading } = useAuth();
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!user || user.role !== "admin") return;
    adminApi
      .users()
      .then(setUsers)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [user]);

  if (authLoading) return null;

  if (!user || user.role !== "admin") {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Access Denied</h1>
        <p className="text-gray-600">Admin access required.</p>
      </div>
    );
  }

  const handleRoleChange = async (userId: string, newRole: string) => {
    try {
      const updated = await adminApi.changeRole(userId, newRole);
      setUsers((prev) => prev.map((u) => (u.id === userId ? updated : u)));
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to change role");
    }
  };

  const handleSuspend = async (userId: string) => {
    if (!confirm("Suspend this user?")) return;
    try {
      const updated = await adminApi.suspendUser(userId);
      setUsers((prev) => prev.map((u) => (u.id === userId ? updated : u)));
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to suspend user");
    }
  };

  const handleActivate = async (userId: string) => {
    try {
      const updated = await adminApi.activateUser(userId);
      setUsers((prev) => prev.map((u) => (u.id === userId ? updated : u)));
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to activate user");
    }
  };

  return (
    <div className="max-w-6xl mx-auto px-4 py-8">
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-3xl font-bold">User Management</h1>
        <Link href="/admin" className="text-blue-600 hover:underline text-sm">
          &larr; Back to Admin
        </Link>
      </div>

      {loading ? (
        <div className="text-center py-16">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto" />
        </div>
      ) : (
        <div className="bg-white rounded-xl shadow-sm border">
          <div className="border-b px-6 py-4">
            <h2 className="text-lg font-semibold">All Users ({users.length})</h2>
          </div>
          <div className="divide-y">
            {users.map((u) => (
              <div key={u.id} className="px-6 py-4 flex items-center justify-between">
                <div>
                  <div className="flex items-center gap-3">
                    <h3 className="font-medium">{u.name}</h3>
                    <span className={`px-2 py-0.5 rounded text-xs font-medium ${roleColors[u.role] || "bg-gray-100"}`}>
                      {u.role}
                    </span>
                    <span className={`px-2 py-0.5 rounded text-xs font-medium ${statusColors[u.status] || "bg-gray-100"}`}>
                      {u.status}
                    </span>
                  </div>
                  <p className="text-sm text-gray-500">
                    {u.email} &middot; {u.auth_provider} &middot; {new Date(u.created_at).toLocaleDateString()}
                  </p>
                </div>
                <div className="flex gap-2">
                  {u.role === "user" ? (
                    <button
                      onClick={() => handleRoleChange(u.id, "admin")}
                      className="px-3 py-1.5 border border-purple-300 text-purple-600 rounded-lg text-xs hover:bg-purple-50"
                    >
                      Make Admin
                    </button>
                  ) : (
                    <button
                      onClick={() => handleRoleChange(u.id, "user")}
                      className="px-3 py-1.5 border border-gray-300 text-gray-600 rounded-lg text-xs hover:bg-gray-50"
                    >
                      Remove Admin
                    </button>
                  )}
                  {u.status === "active" ? (
                    <button
                      onClick={() => handleSuspend(u.id)}
                      className="px-3 py-1.5 border border-red-300 text-red-600 rounded-lg text-xs hover:bg-red-50"
                    >
                      Suspend
                    </button>
                  ) : (
                    <button
                      onClick={() => handleActivate(u.id)}
                      className="px-3 py-1.5 bg-green-600 text-white rounded-lg text-xs hover:bg-green-700"
                    >
                      Activate
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

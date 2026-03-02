"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { useAuth } from "@/components/auth-provider";
import { authApi, programsApi, deploymentsApi } from "@/lib/api";
import { Program, Deployment } from "@/lib/types";

export default function ProfilePage() {
  const { user, loading: authLoading, login } = useAuth();
  const [programs, setPrograms] = useState<Program[]>([]);
  const [deployments, setDeployments] = useState<Deployment[]>([]);
  const [editingName, setEditingName] = useState(false);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  useEffect(() => {
    if (!user) return;
    setName(user.name);
    programsApi.list().then(setPrograms).catch(() => {});
    deploymentsApi.list().then(setDeployments).catch(() => {});
  }, [user]);

  if (authLoading) return null;

  if (!user) {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Login Required</h1>
        <Link href="/login" className="text-blue-600 hover:underline">Go to Login</Link>
      </div>
    );
  }

  const handleSaveName = async () => {
    if (!name.trim()) return;
    setSaving(true);
    setError("");
    setSuccess("");
    try {
      const data = await authApi.updateProfile({ name: name.trim() });
      login(localStorage.getItem("session_token")!, data.user);
      setEditingName(false);
      setSuccess("Name updated successfully");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to update");
    } finally {
      setSaving(false);
    }
  };

  const activePrograms = programs.filter((p) => p.status === "active");
  const pendingPrograms = programs.filter((p) => p.status === "pending");

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <h1 className="text-3xl font-bold mb-8">Profile</h1>

      {error && (
        <div className="p-3 bg-red-50 text-red-600 text-sm rounded-lg mb-4">{error}</div>
      )}
      {success && (
        <div className="p-3 bg-green-50 text-green-700 text-sm rounded-lg mb-4">{success}</div>
      )}

      {/* User Info */}
      <div className="bg-white rounded-xl shadow-sm border p-6 mb-6">
        <h2 className="text-lg font-semibold mb-4">Account</h2>
        <dl className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <dt className="text-sm text-gray-500">Name</dt>
              {editingName ? (
                <div className="flex items-center gap-2 mt-1">
                  <input
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    className="px-2 py-1 border border-gray-300 rounded-lg text-sm"
                  />
                  <button
                    onClick={handleSaveName}
                    disabled={saving}
                    className="px-3 py-1 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700 disabled:opacity-50"
                  >
                    {saving ? "..." : "Save"}
                  </button>
                  <button
                    onClick={() => {
                      setEditingName(false);
                      setName(user.name);
                    }}
                    className="px-3 py-1 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <dd className="font-medium flex items-center gap-2">
                  {user.name}
                  <button
                    onClick={() => setEditingName(true)}
                    className="text-blue-600 text-xs hover:underline"
                  >
                    Edit
                  </button>
                </dd>
              )}
            </div>
          </div>
          <div>
            <dt className="text-sm text-gray-500">Email</dt>
            <dd className="font-medium">{user.email}</dd>
          </div>
          <div>
            <dt className="text-sm text-gray-500">Auth Provider</dt>
            <dd className="font-medium capitalize">{user.authProvider || "email"}</dd>
          </div>
          <div>
            <dt className="text-sm text-gray-500">Role</dt>
            <dd className="font-medium capitalize">{user.role}</dd>
          </div>
        </dl>
      </div>

      {/* Programs Summary */}
      <div className="bg-white rounded-xl shadow-sm border p-6 mb-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">My Programs</h2>
          <Link href="/creator" className="text-blue-600 text-sm hover:underline">
            View All
          </Link>
        </div>

        <div className="grid grid-cols-3 gap-4 text-center">
          <div className="p-4 bg-gray-50 rounded-lg">
            <p className="text-2xl font-bold">{programs.length}</p>
            <p className="text-sm text-gray-500">Total</p>
          </div>
          <div className="p-4 bg-green-50 rounded-lg">
            <p className="text-2xl font-bold text-green-600">{activePrograms.length}</p>
            <p className="text-sm text-gray-500">Active</p>
          </div>
          <div className="p-4 bg-yellow-50 rounded-lg">
            <p className="text-2xl font-bold text-yellow-600">{pendingPrograms.length}</p>
            <p className="text-sm text-gray-500">Pending</p>
          </div>
        </div>

        {programs.length > 0 && (
          <div className="mt-4 space-y-2">
            {programs.slice(0, 3).map((p) => (
              <Link
                key={p.id}
                href={`/creator/${p.id}`}
                className="block p-3 border rounded-lg hover:bg-gray-50 text-sm"
              >
                <span className="font-medium">{p.name}</span>
                <span className="text-gray-400 ml-2">{p.program_id}</span>
              </Link>
            ))}
            {programs.length > 3 && (
              <Link href="/creator" className="block text-center text-blue-600 text-sm hover:underline pt-2">
                +{programs.length - 3} more
              </Link>
            )}
          </div>
        )}
      </div>

      {/* Deployments Summary */}
      <div className="bg-white rounded-xl shadow-sm border p-6 mb-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">My Deployments</h2>
          <Link href="/deployments" className="text-blue-600 text-sm hover:underline">
            View All
          </Link>
        </div>

        <div className="grid grid-cols-3 gap-4 text-center">
          <div className="p-4 bg-gray-50 rounded-lg">
            <p className="text-2xl font-bold">{deployments.length}</p>
            <p className="text-sm text-gray-500">Total</p>
          </div>
          <div className="p-4 bg-green-50 rounded-lg">
            <p className="text-2xl font-bold text-green-600">
              {deployments.filter((d) => d.status === "active").length}
            </p>
            <p className="text-sm text-gray-500">Active</p>
          </div>
          <div className="p-4 bg-yellow-50 rounded-lg">
            <p className="text-2xl font-bold text-yellow-600">
              {deployments.filter((d) => d.status === "configured").length}
            </p>
            <p className="text-sm text-gray-500">Configured</p>
          </div>
        </div>

        {deployments.length > 0 && (
          <div className="mt-4 space-y-2">
            {deployments.slice(0, 3).map((d) => (
              <Link
                key={d.id}
                href={`/deployments/${d.id}`}
                className="block p-3 border rounded-lg hover:bg-gray-50 text-sm"
              >
                <span className="font-medium">{d.name}</span>
                <span className="text-gray-400 ml-2">{d.program_name || d.program_id}</span>
              </Link>
            ))}
            {deployments.length > 3 && (
              <Link href="/deployments" className="block text-center text-blue-600 text-sm hover:underline pt-2">
                +{deployments.length - 3} more
              </Link>
            )}
          </div>
        )}
      </div>

      {/* Quick Actions */}
      <div className="bg-white rounded-xl shadow-sm border p-6">
        <h2 className="text-lg font-semibold mb-4">Quick Actions</h2>
        <div className="flex flex-wrap gap-3">
          <Link
            href="/creator/new"
            className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700"
          >
            Create New Program
          </Link>
          <Link
            href="/store"
            className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
          >
            Browse Store
          </Link>
          <Link
            href="/deployments"
            className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
          >
            My Deployments
          </Link>
        </div>
      </div>
    </div>
  );
}

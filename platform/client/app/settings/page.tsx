"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { hostsApi } from "@/lib/api";
import { useAuth } from "@/components/auth-provider";
import { Host } from "@/lib/types";

export default function SettingsPage() {
  const { user } = useAuth();
  const [hosts, setHosts] = useState<Host[]>([]);
  const [loading, setLoading] = useState(true);
  const [showAdd, setShowAdd] = useState(false);

  // Add host form
  const [name, setName] = useState("");
  const [hostname, setHostname] = useState("");
  const [port, setPort] = useState("22");
  const [username, setUsername] = useState("root");
  const [privateKey, setPrivateKey] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  // Test results
  const [testResults, setTestResults] = useState<Record<string, { ok: boolean; docker: boolean; message: string }>>({});
  const [testing, setTesting] = useState<string | null>(null);

  useEffect(() => {
    if (!user) return;
    hostsApi
      .list()
      .then(setHosts)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [user]);

  const handleAdd = async () => {
    if (!name.trim() || !hostname.trim() || !username.trim()) {
      setError("Name, hostname, and username are required");
      return;
    }
    if (!privateKey.trim()) {
      setError("SSH private key is required");
      return;
    }
    setSaving(true);
    setError("");
    try {
      const host = await hostsApi.create({
        name: name.trim(),
        hostname: hostname.trim(),
        port: parseInt(port) || 22,
        username: username.trim(),
        authMethod: "key",
        privateKey: privateKey,
      });
      setHosts((prev) => [host, ...prev]);
      setShowAdd(false);
      setName("");
      setHostname("");
      setPort("22");
      setUsername("root");
      setPrivateKey("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to add host");
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async (hostId: string) => {
    setTesting(hostId);
    try {
      const result = await hostsApi.test(hostId);
      setTestResults((prev) => ({ ...prev, [hostId]: result }));
      // Refresh host list to get updated status
      const updatedHosts = await hostsApi.list();
      setHosts(updatedHosts);
    } catch (err) {
      setTestResults((prev) => ({
        ...prev,
        [hostId]: { ok: false, docker: false, message: err instanceof Error ? err.message : "Test failed" },
      }));
    } finally {
      setTesting(null);
    }
  };

  const handleDelete = async (hostId: string) => {
    if (!confirm("Remove this host?")) return;
    try {
      await hostsApi.remove(hostId);
      setHosts((prev) => prev.filter((h) => h.id !== hostId));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete");
    }
  };

  const handleKeyFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => setPrivateKey(reader.result as string);
    reader.readAsText(file);
  };

  if (!user) {
    return (
      <div className="max-w-4xl mx-auto px-4 py-16 text-center">
        <Link href="/login" className="text-blue-600 hover:underline">Login required</Link>
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      <h1 className="text-2xl font-bold mb-6">Settings</h1>

      {/* Remote Hosts */}
      <div className="bg-white rounded-xl border p-6">
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-lg font-semibold">Remote Servers</h2>
            <p className="text-sm text-gray-500">
              Add SSH servers to deploy L2 chains remotely using pre-built Docker images.
            </p>
          </div>
          <button
            onClick={() => setShowAdd(!showAdd)}
            className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700"
          >
            {showAdd ? "Cancel" : "Add Server"}
          </button>
        </div>

        {/* Add host form */}
        {showAdd && (
          <div className="border rounded-lg p-4 mb-4 bg-gray-50 space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">Name *</label>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="Production Server"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">Hostname / IP *</label>
                <input
                  type="text"
                  value={hostname}
                  onChange={(e) => setHostname(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="192.168.1.100"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">Username *</label>
                <input
                  type="text"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="root"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">SSH Port</label>
                <input
                  type="number"
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="22"
                />
              </div>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">SSH Private Key *</label>
              <div className="flex gap-2 mb-2">
                <label className="px-3 py-1.5 border rounded-lg text-sm cursor-pointer hover:bg-gray-100">
                  Upload Key File
                  <input type="file" className="hidden" onChange={handleKeyFile} accept=".pem,.key,*" />
                </label>
                <span className="text-xs text-gray-400 self-center">or paste below</span>
              </div>
              <textarea
                value={privateKey}
                onChange={(e) => setPrivateKey(e.target.value)}
                className="w-full px-3 py-2 border rounded-lg text-sm font-mono h-32 focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                placeholder="-----BEGIN OPENSSH PRIVATE KEY-----&#10;..."
              />
            </div>
            {error && <p className="text-sm text-red-600">{error}</p>}
            <button
              onClick={handleAdd}
              disabled={saving}
              className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700 disabled:opacity-50"
            >
              {saving ? "Adding..." : "Add Server"}
            </button>
          </div>
        )}

        {/* Host list */}
        {loading ? (
          <div className="flex justify-center py-8">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600" />
          </div>
        ) : hosts.length === 0 ? (
          <div className="text-center py-8 text-gray-400 text-sm">
            No remote servers added yet.
          </div>
        ) : (
          <div className="space-y-3">
            {hosts.map((host) => (
              <div key={host.id} className="border rounded-lg p-4 flex items-center justify-between">
                <div>
                  <div className="font-medium">{host.name}</div>
                  <div className="text-sm text-gray-500">
                    {host.username}@{host.hostname}:{host.port}
                  </div>
                  <div className="flex items-center gap-2 mt-1">
                    <span
                      className={`px-2 py-0.5 rounded text-xs font-medium ${
                        host.status === "active"
                          ? "bg-green-100 text-green-700"
                          : host.status === "no_docker"
                          ? "bg-yellow-100 text-yellow-700"
                          : host.status === "error"
                          ? "bg-red-100 text-red-700"
                          : "bg-gray-100 text-gray-500"
                      }`}
                    >
                      {host.status === "active" ? "Ready" : host.status === "no_docker" ? "No Docker" : host.status === "error" ? "Error" : "Not tested"}
                    </span>
                    {host.last_tested && (
                      <span className="text-xs text-gray-400">
                        Tested: {new Date(host.last_tested).toLocaleString()}
                      </span>
                    )}
                  </div>
                  {testResults[host.id] && (
                    <p className={`text-xs mt-1 ${testResults[host.id].ok ? "text-green-600" : "text-red-600"}`}>
                      {testResults[host.id].message}
                    </p>
                  )}
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleTest(host.id)}
                    disabled={testing === host.id}
                    className="px-3 py-1.5 border rounded-lg text-sm hover:bg-gray-50 disabled:opacity-50"
                  >
                    {testing === host.id ? "Testing..." : "Test"}
                  </button>
                  <button
                    onClick={() => handleDelete(host.id)}
                    className="px-3 py-1.5 border border-red-200 text-red-600 rounded-lg text-sm hover:bg-red-50"
                  >
                    Remove
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

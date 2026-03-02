"use client";

import { useState, useEffect, useRef } from "react";
import { useParams } from "next/navigation";
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

export default function ProgramDetailPage() {
  const params = useParams();
  const { user, loading: authLoading } = useAuth();
  const [program, setProgram] = useState<Program | null>(null);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState("");
  const [uploadMsg, setUploadMsg] = useState("");
  const elfInputRef = useRef<HTMLInputElement>(null);
  const vkSp1InputRef = useRef<HTMLInputElement>(null);
  const vkRisc0InputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!user || !params.id) return;
    programsApi
      .get(params.id as string)
      .then((data) => {
        setProgram(data);
        setName(data.name);
        setDescription(data.description || "");
      })
      .catch((err) => setError(err instanceof Error ? err.message : "Failed to load"))
      .finally(() => setLoading(false));
  }, [user, params.id]);

  if (authLoading) return null;

  if (!user) {
    return (
      <div className="max-w-3xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Login Required</h1>
        <Link href="/login" className="text-blue-600 hover:underline">Go to Login</Link>
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
        <h1 className="text-xl font-bold text-red-600 mb-4">Error</h1>
        <p className="text-gray-600 mb-4">{error}</p>
        <Link href="/creator" className="text-blue-600 hover:underline">Back to My Programs</Link>
      </div>
    );
  }

  const handleSave = async () => {
    setSaving(true);
    setError("");
    try {
      const updated = await programsApi.update(program.id, { name, description });
      setProgram(updated);
      setEditing(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!confirm("Are you sure you want to deactivate this program?")) return;
    try {
      await programsApi.remove(program.id);
      window.location.href = "/creator";
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to deactivate");
    }
  };

  const handleElfUpload = async (file: File) => {
    setUploading("elf");
    setUploadMsg("");
    setError("");
    try {
      const result = await programsApi.uploadElf(program.id, file);
      setProgram(result.program);
      setUploadMsg(`ELF uploaded (${(result.upload.size / 1024 / 1024).toFixed(1)}MB, hash: ${result.upload.hash.slice(0, 16)}...)`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "ELF upload failed");
    } finally {
      setUploading("");
    }
  };

  const handleVkUpload = async () => {
    const sp1File = vkSp1InputRef.current?.files?.[0];
    const risc0File = vkRisc0InputRef.current?.files?.[0];
    if (!sp1File && !risc0File) return;

    setUploading("vk");
    setUploadMsg("");
    setError("");
    try {
      const result = await programsApi.uploadVk(program.id, {
        sp1: sp1File,
        risc0: risc0File,
      });
      setProgram(result.program);
      const keys = Object.keys(result.upload).join(", ");
      setUploadMsg(`VK uploaded: ${keys}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "VK upload failed");
    } finally {
      setUploading("");
    }
  };

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <Link href="/creator" className="text-blue-600 hover:underline text-sm mb-4 inline-block">
        &larr; Back to My Programs
      </Link>

      <div className="bg-white rounded-xl shadow-sm border p-8">
        {error && (
          <div className="p-3 bg-red-50 text-red-600 text-sm rounded-lg mb-4">{error}</div>
        )}
        {uploadMsg && (
          <div className="p-3 bg-green-50 text-green-700 text-sm rounded-lg mb-4">{uploadMsg}</div>
        )}

        <div className="flex items-start justify-between mb-6">
          <div>
            {editing ? (
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="text-2xl font-bold border border-gray-300 rounded-lg px-2 py-1 mb-1"
              />
            ) : (
              <h1 className="text-2xl font-bold">{program.name}</h1>
            )}
            <p className="text-gray-500">{program.program_id}</p>
          </div>
          <span className={`px-3 py-1 rounded text-sm font-medium ${statusColors[program.status]}`}>
            {program.status}
          </span>
        </div>

        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Description</h2>
          {editing ? (
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={4}
              className="w-full px-3 py-2 border border-gray-300 rounded-lg resize-none"
            />
          ) : (
            <p className="text-gray-700 whitespace-pre-wrap">
              {program.description || "No description"}
            </p>
          )}
        </div>

        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-3">Details</h2>
          <dl className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <dt className="text-gray-500">Category</dt>
              <dd className="font-medium capitalize">{program.category}</dd>
            </div>
            <div>
              <dt className="text-gray-500">Type ID</dt>
              <dd className="font-medium">{program.program_type_id ?? "Pending"}</dd>
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
              <dd className="font-medium">{new Date(program.created_at).toLocaleDateString()}</dd>
            </div>
            {program.approved_at && (
              <div>
                <dt className="text-gray-500">Approved</dt>
                <dd className="font-medium">{new Date(program.approved_at).toLocaleDateString()}</dd>
              </div>
            )}
          </dl>
        </div>

        {/* ELF & VK Upload Section */}
        <div className="border-t pt-6 mb-6">
          <h2 className="text-lg font-semibold mb-4">Artifacts</h2>

          {/* ELF Upload */}
          <div className="mb-4">
            <div className="flex items-center justify-between mb-2">
              <label className="text-sm font-medium text-gray-700">ELF Binary</label>
              {program.elf_hash && (
                <span className="text-xs text-green-600 font-mono">
                  {program.elf_hash.slice(0, 16)}...
                </span>
              )}
            </div>
            <div className="flex gap-2">
              <input
                ref={elfInputRef}
                type="file"
                className="hidden"
                onChange={(e) => {
                  if (e.target.files?.[0]) handleElfUpload(e.target.files[0]);
                }}
              />
              <button
                onClick={() => elfInputRef.current?.click()}
                disabled={uploading === "elf"}
                className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50 disabled:opacity-50"
              >
                {uploading === "elf" ? "Uploading..." : program.elf_hash ? "Replace ELF" : "Upload ELF"}
              </button>
              {!program.elf_hash && (
                <p className="text-xs text-gray-400 self-center">
                  Upload the compiled ELF binary for your guest program
                </p>
              )}
            </div>
          </div>

          {/* VK Upload */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <label className="text-sm font-medium text-gray-700">Verification Keys</label>
            </div>
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <span className="text-xs text-gray-500 w-12">SP1:</span>
                <input ref={vkSp1InputRef} type="file" className="text-sm flex-1" />
                {program.vk_sp1 && (
                  <span className="text-xs text-green-600 font-mono">
                    {program.vk_sp1.slice(0, 12)}...
                  </span>
                )}
              </div>
              <div className="flex items-center gap-3">
                <span className="text-xs text-gray-500 w-12">RISC0:</span>
                <input ref={vkRisc0InputRef} type="file" className="text-sm flex-1" />
                {program.vk_risc0 && (
                  <span className="text-xs text-green-600 font-mono">
                    {program.vk_risc0.slice(0, 12)}...
                  </span>
                )}
              </div>
              <button
                onClick={handleVkUpload}
                disabled={uploading === "vk"}
                className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50 disabled:opacity-50"
              >
                {uploading === "vk" ? "Uploading..." : "Upload VKs"}
              </button>
            </div>
          </div>
        </div>

        <div className="border-t pt-6 flex gap-3">
          {editing ? (
            <>
              <button
                onClick={handleSave}
                disabled={saving}
                className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50"
              >
                {saving ? "Saving..." : "Save Changes"}
              </button>
              <button
                onClick={() => {
                  setEditing(false);
                  setName(program.name);
                  setDescription(program.description || "");
                }}
                className="px-4 py-2 border border-gray-300 rounded-lg hover:bg-gray-50"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              <button
                onClick={() => setEditing(true)}
                className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700"
              >
                Edit
              </button>
              {program.status !== "disabled" && (
                <button
                  onClick={handleDelete}
                  className="px-4 py-2 border border-red-300 text-red-600 rounded-lg hover:bg-red-50"
                >
                  Deactivate
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

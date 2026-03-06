"use client";

import { useState, useEffect } from "react";
import { fsApi } from "@/lib/api";

interface DirEntry {
  name: string;
  path: string;
}

interface DirectoryPickerProps {
  open: boolean;
  onClose: () => void;
  onSelect: (path: string) => void;
  initialPath?: string;
}

export default function DirectoryPicker({ open, onClose, onSelect, initialPath }: DirectoryPickerProps) {
  const [currentPath, setCurrentPath] = useState("");
  const [parentPath, setParentPath] = useState<string | null>(null);
  const [dirs, setDirs] = useState<DirEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (open) {
      browse(initialPath || "");
    }
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  const browse = async (dirPath: string) => {
    setLoading(true);
    setError("");
    try {
      const data = await fsApi.browse(dirPath || undefined);
      setCurrentPath(data.current);
      setParentPath(data.parent);
      setDirs(data.dirs);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to browse");
    } finally {
      setLoading(false);
    }
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white rounded-xl shadow-xl w-full max-w-lg mx-4">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b">
          <h3 className="font-semibold">Select Directory</h3>
          <button onClick={onClose} className="text-gray-400 hover:text-gray-600 text-xl">&times;</button>
        </div>

        {/* Current path */}
        <div className="px-4 py-2 bg-gray-50 border-b">
          <p className="text-sm font-mono text-gray-700 truncate">{currentPath}</p>
        </div>

        {/* Directory list */}
        <div className="h-80 overflow-y-auto">
          {loading ? (
            <div className="flex justify-center py-8">
              <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600" />
            </div>
          ) : error ? (
            <div className="p-4 text-sm text-red-600">{error}</div>
          ) : (
            <div>
              {parentPath && (
                <button
                  onClick={() => browse(parentPath)}
                  className="w-full text-left px-4 py-2 hover:bg-gray-50 flex items-center gap-2 text-sm border-b"
                >
                  <span className="text-gray-400">&#8593;</span>
                  <span className="text-gray-500">..</span>
                </button>
              )}
              {dirs.length === 0 ? (
                <div className="p-4 text-sm text-gray-400 text-center">No subdirectories</div>
              ) : (
                dirs.map((dir) => (
                  <button
                    key={dir.path}
                    onClick={() => browse(dir.path)}
                    className="w-full text-left px-4 py-2 hover:bg-blue-50 flex items-center gap-2 text-sm border-b border-gray-100"
                  >
                    <span className="text-blue-500">&#128193;</span>
                    <span>{dir.name}</span>
                  </button>
                ))
              )}
            </div>
          )}
        </div>

        {/* Actions */}
        <div className="flex items-center justify-between px-4 py-3 border-t bg-gray-50 rounded-b-xl">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
          >
            Cancel
          </button>
          <button
            onClick={() => { onSelect(currentPath); onClose(); }}
            className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700"
          >
            Select This Directory
          </button>
        </div>
      </div>
    </div>
  );
}

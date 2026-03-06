"use client";

import { useState, useEffect, useRef } from "react";
import { deploymentsApi } from "@/lib/api";

interface LogViewerProps {
  deploymentId: string;
  initialService?: string;
}

const SERVICES = [
  { value: "", label: "All Services" },
  { value: "tokamak-app-l1", label: "L1 Node" },
  { value: "tokamak-app-l2", label: "L2 Node" },
  { value: "tokamak-app-prover", label: "Prover" },
  { value: "tokamak-app-deployer", label: "Deployer" },
  { value: "bridge-ui", label: "Bridge UI" },
  { value: "backend-l1", label: "Explorer L1 Backend" },
  { value: "backend-l2", label: "Explorer L2 Backend" },
  { value: "proxy", label: "Explorer Proxy" },
  { value: "db", label: "Explorer DB" },
];

export default function LogViewer({ deploymentId, initialService }: LogViewerProps) {
  const [service, setService] = useState(initialService || "");
  const [lines, setLines] = useState<string[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [search, setSearch] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const logRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  // Fetch initial logs
  useEffect(() => {
    const fetchLogs = async () => {
      try {
        const data = await deploymentsApi.status(deploymentId);
        if (data.phase === "configured") return;

        // Fetch non-streaming logs first
        const response = await fetch(
          `${process.env.NEXT_PUBLIC_API_URL || "http://localhost:5001"}/api/deployments/${deploymentId}/logs?service=${service}&tail=200`,
          {
            headers: {
              Authorization: `Bearer ${localStorage.getItem("session_token")}`,
            },
          }
        );
        const logData = await response.json();
        if (logData.logs) {
          setLines(logData.logs.split("\n").filter(Boolean));
        }
      } catch {
        // Ignore errors
      }
    };

    fetchLogs();
  }, [deploymentId, service]);

  // Start/stop streaming
  useEffect(() => {
    if (!streaming) {
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
        eventSourceRef.current = null;
      }
      return;
    }

    const url = deploymentsApi.logsUrl(deploymentId, service || undefined);
    const es = new EventSource(url);
    eventSourceRef.current = es;

    es.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data);
        if (data.line) {
          setLines((prev) => [...prev.slice(-2000), data.line]); // Keep last 2000 lines
        }
      } catch {
        // Ignore
      }
    };

    es.onerror = () => {
      setStreaming(false);
    };

    return () => {
      es.close();
    };
  }, [streaming, deploymentId, service]);

  // Auto scroll
  useEffect(() => {
    if (autoScroll && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [lines, autoScroll]);

  const filteredLines = search
    ? lines.filter((l) => l.toLowerCase().includes(search.toLowerCase()))
    : lines;

  return (
    <div className="space-y-3">
      {/* Controls */}
      <div className="flex items-center gap-3">
        <select
          value={service}
          onChange={(e) => {
            setService(e.target.value);
            setLines([]);
          }}
          className="px-3 py-1.5 border rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
        >
          {SERVICES.map((s) => (
            <option key={s.value} value={s.value}>{s.label}</option>
          ))}
        </select>

        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search logs..."
          className="flex-1 px-3 py-1.5 border rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
        />

        <button
          onClick={() => setStreaming(!streaming)}
          className={`px-3 py-1.5 rounded-lg text-sm font-medium ${
            streaming
              ? "bg-red-100 text-red-700 hover:bg-red-200"
              : "bg-green-100 text-green-700 hover:bg-green-200"
          }`}
        >
          {streaming ? "Stop" : "Stream"}
        </button>

        <label className="flex items-center gap-1.5 text-sm text-gray-500">
          <input
            type="checkbox"
            checked={autoScroll}
            onChange={(e) => setAutoScroll(e.target.checked)}
            className="rounded"
          />
          Auto-scroll
        </label>
      </div>

      {/* Log output */}
      <div
        ref={logRef}
        className="bg-gray-900 text-gray-300 rounded-lg p-4 h-96 overflow-y-auto font-mono text-xs leading-5"
      >
        {filteredLines.length === 0 ? (
          <div className="text-gray-500 text-center py-8">
            {lines.length === 0 ? "No logs available" : "No matching lines"}
          </div>
        ) : (
          filteredLines.map((line, i) => (
            <div key={i} className="hover:bg-gray-800 px-1 -mx-1 rounded">
              {search ? (
                <span
                  dangerouslySetInnerHTML={{
                    __html: line.replace(
                      new RegExp(`(${search.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi"),
                      '<mark class="bg-yellow-400 text-black">$1</mark>'
                    ),
                  }}
                />
              ) : (
                line
              )}
            </div>
          ))
        )}
      </div>

      <div className="text-xs text-gray-400 text-right">
        {filteredLines.length} / {lines.length} lines
      </div>
    </div>
  );
}

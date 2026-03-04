"use client";

import { useState, useEffect, useRef } from "react";
import { DeploymentEvent } from "@/lib/types";

const LOCAL_STEPS = [
  { phase: "building", label: "Building Docker Images" },
  { phase: "l1_starting", label: "Starting L1 Node" },
  { phase: "deploying_contracts", label: "Deploying Contracts" },
  { phase: "l2_starting", label: "Starting L2 Node" },
  { phase: "starting_prover", label: "Starting Prover" },
  { phase: "running", label: "Running" },
];

const REMOTE_STEPS = [
  { phase: "pulling", label: "Pulling Docker Images" },
  { phase: "l1_starting", label: "Starting L1 Node" },
  { phase: "deploying_contracts", label: "Deploying Contracts" },
  { phase: "l2_starting", label: "Starting L2 Node" },
  { phase: "starting_prover", label: "Starting Prover" },
  { phase: "running", label: "Running" },
];

interface DeploymentProgressProps {
  deploymentId: string;
  eventsUrl: string;
  remote?: boolean;
  onComplete?: (event: DeploymentEvent) => void;
  onError?: (error: string) => void;
}

export default function DeploymentProgress({
  deploymentId,
  eventsUrl,
  remote = false,
  onComplete,
  onError,
}: DeploymentProgressProps) {
  const STEPS = remote ? REMOTE_STEPS : LOCAL_STEPS;
  const [currentPhase, setCurrentPhase] = useState("configured");
  const [message, setMessage] = useState("Preparing deployment...");
  const [events, setEvents] = useState<DeploymentEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  useEffect(() => {
    const es = new EventSource(eventsUrl);
    eventSourceRef.current = es;

    es.onmessage = (e) => {
      try {
        const data: DeploymentEvent = JSON.parse(e.data);
        setEvents((prev) => [...prev, data]);

        if (data.phase) {
          setCurrentPhase(data.phase);
        }
        if (data.message) {
          setMessage(data.message);
        }
        if (data.event === "error") {
          setError(data.message || "Deployment failed");
          onError?.(data.message || "Deployment failed");
          es.close();
        }
        if (data.phase === "running") {
          onComplete?.(data);
          es.close();
        }
      } catch {
        // Ignore parse errors
      }
    };

    es.onerror = () => {
      // EventSource will auto-reconnect, but if we're done, close it
      if (currentPhase === "running" || error) {
        es.close();
      }
    };

    return () => {
      es.close();
    };
  }, [eventsUrl]); // eslint-disable-line react-hooks/exhaustive-deps

  const currentStepIndex = STEPS.findIndex((s) => s.phase === currentPhase);

  return (
    <div className="space-y-6">
      {/* Step progress */}
      <div className="space-y-3">
        {STEPS.map((step, i) => {
          const isComplete = i < currentStepIndex || currentPhase === "running";
          const isCurrent = step.phase === currentPhase;
          const isPending = i > currentStepIndex && currentPhase !== "running";

          return (
            <div key={step.phase} className="flex items-center gap-3">
              {/* Step indicator */}
              <div
                className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold shrink-0 ${
                  isComplete
                    ? "bg-green-100 text-green-700"
                    : isCurrent
                    ? "bg-blue-600 text-white"
                    : "bg-gray-100 text-gray-400"
                }`}
              >
                {isComplete ? (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                ) : isCurrent ? (
                  <div className="w-3 h-3 border-2 border-white border-t-transparent rounded-full animate-spin" />
                ) : (
                  <span>{i + 1}</span>
                )}
              </div>

              {/* Step label */}
              <div className={`text-sm ${isCurrent ? "font-semibold text-gray-900" : isComplete ? "text-green-700" : "text-gray-400"}`}>
                {step.label}
              </div>
            </div>
          );
        })}
      </div>

      {/* Current message */}
      {message && !error && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-3 text-sm text-blue-800">
          {message}
        </div>
      )}

      {/* Error */}
      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-3 text-sm text-red-800">
          <span className="font-medium">Error: </span>{error}
        </div>
      )}

      {/* Completion info */}
      {currentPhase === "running" && events.length > 0 && (() => {
        const lastEvent = events[events.length - 1];
        return (
          <div className="bg-green-50 border border-green-200 rounded-lg p-4 space-y-2">
            <p className="text-sm font-semibold text-green-800">Deployment is running!</p>
            {lastEvent.l1Rpc && (
              <p className="text-sm text-green-700">
                L1 RPC: <code className="bg-green-100 px-1 rounded">{lastEvent.l1Rpc}</code>
              </p>
            )}
            {lastEvent.l2Rpc && (
              <p className="text-sm text-green-700">
                L2 RPC: <code className="bg-green-100 px-1 rounded">{lastEvent.l2Rpc}</code>
              </p>
            )}
            {lastEvent.bridgeAddress && (
              <p className="text-sm text-green-700">
                Bridge: <code className="bg-green-100 px-1 rounded text-xs">{lastEvent.bridgeAddress}</code>
              </p>
            )}
            {lastEvent.proposerAddress && (
              <p className="text-sm text-green-700">
                Proposer: <code className="bg-green-100 px-1 rounded text-xs">{lastEvent.proposerAddress}</code>
              </p>
            )}
          </div>
        );
      })()}

      {/* Event log */}
      <details className="text-xs">
        <summary className="text-gray-400 cursor-pointer hover:text-gray-600">
          Event log ({events.length} events)
        </summary>
        <div className="mt-2 bg-gray-900 text-gray-300 rounded-lg p-3 max-h-48 overflow-y-auto font-mono">
          {events.map((e, i) => (
            <div key={i} className="py-0.5">
              <span className="text-gray-500">{new Date(e.timestamp).toLocaleTimeString()}</span>{" "}
              <span className={e.event === "error" ? "text-red-400" : "text-green-400"}>[{e.event}]</span>{" "}
              {e.message || e.phase}
            </div>
          ))}
        </div>
      </details>
    </div>
  );
}

"use client";

import { useState, useEffect, useRef } from "react";
import { DeploymentEvent } from "@/lib/types";

const MAX_BUILD_LOG_LINES = 200;

// Estimated duration per phase (seconds) — used for time hints
const PHASE_ESTIMATES: Record<string, { min: number; max: number }> = {
  checking_docker: { min: 1, max: 5 },
  building: { min: 120, max: 600 },   // 2~10 min (first build much longer)
  pulling: { min: 30, max: 180 },
  l1_starting: { min: 5, max: 30 },
  deploying_contracts: { min: 30, max: 120 },
  l2_starting: { min: 10, max: 60 },
  starting_prover: { min: 5, max: 15 },
  starting_tools: { min: 10, max: 60 },
};

const LOCAL_STEPS = [
  { phase: "checking_docker", label: "Checking Docker" },
  { phase: "building", label: "Building Docker Images" },
  { phase: "l1_starting", label: "Starting L1 Node" },
  { phase: "deploying_contracts", label: "Deploying Contracts" },
  { phase: "l2_starting", label: "Starting L2 Node" },
  { phase: "starting_prover", label: "Starting Prover" },
  { phase: "starting_tools", label: "Starting Tools (Blockscout, Bridge UI)" },
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

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

function formatEstimate(phase: string): string | null {
  const est = PHASE_ESTIMATES[phase];
  if (!est) return null;
  if (est.max <= 10) return null; // don't show for very short phases
  const minStr = formatDuration(est.min);
  const maxStr = formatDuration(est.max);
  return `~${minStr}–${maxStr}`;
}

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
  const [buildLogs, setBuildLogs] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const buildLogRef = useRef<HTMLDivElement | null>(null);

  // Track phase start time and elapsed
  const [phaseStartTime, setPhaseStartTime] = useState<number>(Date.now());
  const [elapsed, setElapsed] = useState(0);
  const [totalElapsed, setTotalElapsed] = useState(0);
  const [deployStartTime] = useState<number>(Date.now());
  // Track completed phase durations
  const [phaseDurations, setPhaseDurations] = useState<Record<string, number>>({});

  // Update elapsed every second
  useEffect(() => {
    if (currentPhase === "running" || currentPhase === "configured" || error) return;
    const interval = setInterval(() => {
      const now = Date.now();
      setElapsed(Math.floor((now - phaseStartTime) / 1000));
      setTotalElapsed(Math.floor((now - deployStartTime) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, [phaseStartTime, currentPhase, error, deployStartTime]);

  // Reset elapsed when phase changes
  const prevPhaseRef = useRef(currentPhase);
  useEffect(() => {
    if (prevPhaseRef.current !== currentPhase) {
      // Record duration of previous phase
      const prevDuration = Math.floor((Date.now() - phaseStartTime) / 1000);
      if (prevPhaseRef.current !== "configured") {
        setPhaseDurations((prev) => ({ ...prev, [prevPhaseRef.current]: prevDuration }));
      }
      prevPhaseRef.current = currentPhase;
      setPhaseStartTime(Date.now());
      setElapsed(0);
    }
  }, [currentPhase, phaseStartTime]);

  // Auto-scroll build log to bottom
  useEffect(() => {
    if (buildLogRef.current) {
      buildLogRef.current.scrollTop = buildLogRef.current.scrollHeight;
    }
  }, [buildLogs]);

  useEffect(() => {
    const es = new EventSource(eventsUrl);
    eventSourceRef.current = es;

    es.onmessage = (e) => {
      try {
        const data: DeploymentEvent = JSON.parse(e.data);

        // Build log lines go to separate state (not events array)
        if (data.event === "log") {
          setBuildLogs((prev) => {
            const next = [...prev, data.message || ""];
            return next.length > MAX_BUILD_LOG_LINES
              ? next.slice(next.length - MAX_BUILD_LOG_LINES)
              : next;
          });
          return;
        }

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
          setTotalElapsed(Math.floor((Date.now() - deployStartTime) / 1000));
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
  const isBuilding = currentPhase === "building";
  const isTerminal = currentPhase === "running" || !!error;

  return (
    <div className="space-y-6">
      {/* Total elapsed time */}
      {!isTerminal && (
        <div className="flex items-center justify-between text-sm text-gray-500">
          <span>Total elapsed: <span className="font-mono font-medium text-gray-700">{formatDuration(totalElapsed)}</span></span>
          {currentPhase !== "configured" && (
            <span className="text-xs text-gray-400">
              Step {currentStepIndex + 1} of {STEPS.length - 1}
            </span>
          )}
        </div>
      )}
      {isTerminal && currentPhase === "running" && (
        <div className="flex items-center gap-2 text-sm text-green-700">
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
          </svg>
          Completed in <span className="font-mono font-medium">{formatDuration(totalElapsed)}</span>
        </div>
      )}

      {/* Step progress */}
      <div className="space-y-3">
        {STEPS.map((step, i) => {
          const isComplete = i < currentStepIndex || currentPhase === "running";
          const isCurrent = step.phase === currentPhase;
          const isPending = i > currentStepIndex && currentPhase !== "running";
          const completedDuration = phaseDurations[step.phase];
          const estimate = formatEstimate(step.phase);

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

              {/* Step label + time info */}
              <div className="flex-1 flex items-center justify-between min-w-0">
                <div className={`text-sm ${isCurrent ? "font-semibold text-gray-900" : isComplete ? "text-green-700" : "text-gray-400"}`}>
                  {step.label}
                </div>
                <div className="text-xs font-mono shrink-0 ml-2">
                  {isCurrent && !isTerminal && (
                    <span className="text-blue-600">{formatDuration(elapsed)}</span>
                  )}
                  {isCurrent && !isTerminal && estimate && (
                    <span className="text-gray-400 ml-1">({estimate})</span>
                  )}
                  {isComplete && completedDuration !== undefined && (
                    <span className="text-green-600">{formatDuration(completedDuration)}</span>
                  )}
                  {isPending && estimate && (
                    <span className="text-gray-300">{estimate}</span>
                  )}
                </div>
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

      {/* Build log — shown during and after build phase */}
      {buildLogs.length > 0 && (
        <details open={isBuilding}>
          <summary className="text-sm text-gray-500 cursor-pointer hover:text-gray-700 font-medium">
            Build output ({buildLogs.length} lines)
          </summary>
          <div
            ref={buildLogRef}
            className="mt-2 bg-gray-900 text-gray-300 rounded-lg p-3 max-h-64 overflow-y-auto font-mono text-xs leading-relaxed"
          >
            {buildLogs.map((line, i) => (
              <div key={i} className="whitespace-pre-wrap break-all">{line}</div>
            ))}
          </div>
        </details>
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

"use client";

import { ContainerStatus } from "@/lib/types";

interface DeploymentStatusBadgeProps {
  phase: string;
  className?: string;
}

const PHASE_STYLES: Record<string, { bg: string; text: string; label: string }> = {
  configured: { bg: "bg-gray-100", text: "text-gray-600", label: "Not deployed" },
  checking_docker: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Checking Docker" },
  building: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Building" },
  pulling: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Pulling Images" },
  l1_starting: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Starting L1" },
  deploying_contracts: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Deploying" },
  l2_starting: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Starting L2" },
  starting_prover: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Starting Prover" },
  starting_tools: { bg: "bg-yellow-100", text: "text-yellow-700", label: "Starting Tools" },
  running: { bg: "bg-green-100", text: "text-green-700", label: "Running" },
  stopped: { bg: "bg-orange-100", text: "text-orange-700", label: "Stopped" },
  error: { bg: "bg-red-100", text: "text-red-700", label: "Error" },
};

export function DeploymentStatusBadge({ phase, className = "" }: DeploymentStatusBadgeProps) {
  const style = PHASE_STYLES[phase] || PHASE_STYLES.configured;
  const isAnimating = ["checking_docker", "building", "pulling", "l1_starting", "deploying_contracts", "l2_starting", "starting_prover", "starting_tools"].includes(phase);

  return (
    <span className={`inline-flex items-center gap-1.5 px-3 py-1 rounded-full text-sm font-medium ${style.bg} ${style.text} ${className}`}>
      {isAnimating && (
        <span className="w-2 h-2 rounded-full bg-current animate-pulse" />
      )}
      {phase === "running" && (
        <span className="w-2 h-2 rounded-full bg-green-500" />
      )}
      {style.label}
    </span>
  );
}

interface ContainerStatusCardProps {
  containers: ContainerStatus[];
}

const SERVICE_LABELS: Record<string, string> = {
  "tokamak-app-l1": "L1 Node",
  "tokamak-app-deployer": "Deployer",
  "tokamak-app-l2": "L2 Node",
  "tokamak-app-prover": "Prover",
};

export function ContainerStatusCards({ containers }: ContainerStatusCardProps) {
  const services = ["tokamak-app-l1", "tokamak-app-l2", "tokamak-app-prover", "tokamak-app-deployer"];

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
      {services.map((service) => {
        // Match by service name or container name (which may include project prefix)
        const container = containers.find(
          (c) => c.Service === service || c.Name?.includes(service.replace("tokamak-app-", ""))
        );
        const isRunning = container?.State === "running";
        const isExited = container?.State === "exited";
        const label = SERVICE_LABELS[service] || service;

        return (
          <div
            key={service}
            className={`rounded-lg border p-3 text-center ${
              isRunning
                ? "border-green-200 bg-green-50"
                : isExited
                ? "border-gray-200 bg-gray-50"
                : "border-gray-100 bg-gray-50"
            }`}
          >
            <div className="text-sm font-medium text-gray-700">{label}</div>
            <div className={`text-xs mt-1 ${isRunning ? "text-green-600" : "text-gray-400"}`}>
              {container ? container.State : "not started"}
            </div>
          </div>
        );
      })}
    </div>
  );
}

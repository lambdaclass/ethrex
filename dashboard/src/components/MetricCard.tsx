import type { RegressionStatus } from "@/types";
import { StatusBadge } from "./StatusBadge";

interface Props {
  readonly label: string;
  readonly value: string;
  readonly status?: RegressionStatus;
}

export function MetricCard({ label, value, status }: Props) {
  return (
    <div className="rounded-lg border border-tokamak-border bg-tokamak-card p-4">
      <p className="text-sm text-slate-400">{label}</p>
      <p className="mt-1 text-2xl font-semibold text-white">{value}</p>
      {status && (
        <div className="mt-2">
          <StatusBadge status={status} />
        </div>
      )}
    </div>
  );
}

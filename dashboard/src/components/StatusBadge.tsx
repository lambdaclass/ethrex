import type { RegressionStatus } from "@/types";

const BADGE_STYLES: Record<RegressionStatus, string> = {
  Stable: "bg-tokamak-green/20 text-tokamak-green",
  Warning: "bg-tokamak-yellow/20 text-tokamak-yellow",
  Regression: "bg-tokamak-red/20 text-tokamak-red",
};

interface Props {
  readonly status: RegressionStatus;
}

export function StatusBadge({ status }: Props) {
  return (
    <span className={`inline-block rounded-full px-2 py-0.5 text-xs font-medium ${BADGE_STYLES[status]}`}>
      {status}
    </span>
  );
}

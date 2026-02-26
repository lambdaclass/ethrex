/** Base URL for dashboard data files (overridden in dev). */
export const DATA_BASE_URL =
  import.meta.env.PUBLIC_DATA_URL ?? "/data";

/** Chart color palette. */
export const COLORS = {
  interpreter: "#6366f1",
  jit: "#22c55e",
  ethrex: "#6366f1",
  geth: "#f97316",
  reth: "#8b5cf6",
  ci_band: "rgba(99, 102, 241, 0.15)",
  grid: "#2a2d3e",
  text: "#94a3b8",
} as const;

/** Status color mapping. */
export const STATUS_COLORS = {
  Stable: "#22c55e",
  Warning: "#eab308",
  Regression: "#ef4444",
} as const;

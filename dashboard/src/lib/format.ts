/** Format nanoseconds into a human-readable duration string. */
export function formatNs(ns: number): string {
  if (ns >= 1_000_000_000) {
    return `${(ns / 1_000_000_000).toFixed(ns >= 10_000_000_000 ? 1 : 2)} s`;
  }
  if (ns >= 1_000_000) {
    return `${(ns / 1_000_000).toFixed(2)} ms`;
  }
  if (ns >= 1_000) {
    return `${(ns / 1_000).toFixed(2)} \u00b5s`;
  }
  return `${ns.toFixed(1)} ns`;
}

/** Format a speedup ratio (e.g. 2.50x), or N/A for null. */
export function formatSpeedup(speedup: number | null): string {
  if (speedup === null) {
    return "N/A";
  }
  return `${speedup.toFixed(2)}x`;
}

/** Format a percentage change with sign (e.g. +25.0%). */
export function formatPercent(pct: number): string {
  const sign = pct >= 0 ? "+" : "";
  return `${sign}${pct.toFixed(1)}%`;
}

/** Truncate a commit hash to 7 characters. */
export function formatCommit(commit: string): string {
  return commit.slice(0, 7);
}

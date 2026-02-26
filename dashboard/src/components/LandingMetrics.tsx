import { useEffect, useState } from "react";
import { fetchIndex, fetchBenchSuite } from "@/lib/data";
import { formatNs, formatCommit } from "@/lib/format";
import { DATA_BASE_URL } from "@/lib/constants";
import { MetricCard } from "./MetricCard";
import { BenchTable } from "./BenchTable";
import type { BenchSuite, DashboardIndex } from "@/types";

export function LandingMetrics() {
  const [index, setIndex] = useState<DashboardIndex | null>(null);
  const [suite, setSuite] = useState<BenchSuite | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const idx = await fetchIndex(DATA_BASE_URL);
        if (cancelled) return;
        setIndex(idx);

        if (idx.runs.length === 0) return;
        const latest = idx.runs[idx.runs.length - 1];
        const benchSuite = await fetchBenchSuite(DATA_BASE_URL, latest.bench);
        if (cancelled) return;
        setSuite(benchSuite);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : "Unknown error");
      }
    }

    load();
    return () => { cancelled = true; };
  }, []);

  if (error) {
    return <p className="text-tokamak-red">Error: {error}</p>;
  }

  if (!suite) {
    return <p className="text-slate-400">Loading...</p>;
  }

  const avgMean = suite.results.reduce(
    (sum, r) => sum + (r.stats?.mean_ns ?? r.total_duration_ns / r.runs),
    0,
  ) / (suite.results.length || 1);

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <MetricCard label="Latest Commit" value={formatCommit(suite.commit)} />
        <MetricCard label="Avg Mean Time" value={formatNs(avgMean)} />
        <MetricCard label="Scenarios" value={String(suite.results.length)} />
      </div>
      <div className="rounded-lg border border-tokamak-border bg-tokamak-card">
        <BenchTable results={suite.results} />
      </div>
    </div>
  );
}

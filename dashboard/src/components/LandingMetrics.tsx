import { useEffect, useState } from "react";
import { fetchIndex, fetchBenchSuite, fetchJitBenchSuite, fetchCrossClientSuite } from "@/lib/data";
import { formatNs, formatCommit, formatSpeedup } from "@/lib/format";
import { DATA_BASE_URL } from "@/lib/constants";
import { MetricCard } from "./MetricCard";
import { BenchTable } from "./BenchTable";
import { JitSpeedupTable } from "./JitSpeedupTable";
import { CrossClientTable } from "./CrossClientTable";
import type { BenchSuite, JitBenchSuite, CrossClientSuite, DashboardIndex } from "@/types";

export function LandingMetrics() {
  const [index, setIndex] = useState<DashboardIndex | null>(null);
  const [suite, setSuite] = useState<BenchSuite | null>(null);
  const [jitSuite, setJitSuite] = useState<JitBenchSuite | null>(null);
  const [crossClient, setCrossClient] = useState<CrossClientSuite | null>(null);
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

        if (latest.jit_bench) {
          const jit = await fetchJitBenchSuite(DATA_BASE_URL, latest.jit_bench);
          if (!cancelled) setJitSuite(jit);
        }

        if (latest.cross_client) {
          const cc = await fetchCrossClientSuite(DATA_BASE_URL, latest.cross_client);
          if (!cancelled) setCrossClient(cc);
        }
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

  const bestSpeedup = jitSuite
    ? jitSuite.results.reduce<{ scenario: string; speedup: number } | null>((best, r) => {
        if (r.speedup === null) return best;
        if (best === null || r.speedup > best.speedup) {
          return { scenario: r.scenario, speedup: r.speedup };
        }
        return best;
      }, null)
    : null;

  return (
    <div className="space-y-8">
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3 lg:grid-cols-4">
        <MetricCard label="Latest Commit" value={formatCommit(suite.commit)} />
        <MetricCard label="Avg Mean Time" value={formatNs(avgMean)} />
        <MetricCard label="Scenarios" value={String(suite.results.length)} />
        {bestSpeedup && (
          <MetricCard
            label="Best JIT Speedup"
            value={`${formatSpeedup(bestSpeedup.speedup)} (${bestSpeedup.scenario})`}
          />
        )}
      </div>

      <section className="space-y-3">
        <h2 className="text-lg font-semibold">Interpreter Benchmarks</h2>
        <div className="rounded-lg border border-tokamak-border bg-tokamak-card">
          <BenchTable results={suite.results} />
        </div>
      </section>

      {jitSuite && (
        <section className="space-y-3">
          <h2 className="text-lg font-semibold">JIT Speedup</h2>
          <div className="rounded-lg border border-tokamak-border bg-tokamak-card">
            <JitSpeedupTable results={jitSuite.results} />
          </div>
        </section>
      )}

      {crossClient && (
        <section className="space-y-3">
          <h2 className="text-lg font-semibold">Cross-Client Comparison</h2>
          <div className="rounded-lg border border-tokamak-border bg-tokamak-card">
            <CrossClientTable scenarios={crossClient.scenarios} />
          </div>
        </section>
      )}
    </div>
  );
}

import { useEffect, useState, useMemo } from "react";
import { fetchIndex, fetchJitBenchSuite, fetchCrossClientSuite } from "@/lib/data";
import { DATA_BASE_URL, COLORS } from "@/lib/constants";
import { formatSpeedup } from "@/lib/format";
import { JitSpeedupTable } from "./JitSpeedupTable";
import { CrossClientTable } from "./CrossClientTable";
import { JitToggle } from "./JitToggle";
import { DateRangePicker, type DateRange } from "./DateRangePicker";
import type { JitBenchSuite, CrossClientSuite, DashboardIndex } from "@/types";
import {
  ResponsiveContainer,
  LineChart,
  Line,
  XAxis,
  YAxis,
  Tooltip,
  CartesianGrid,
  Legend,
} from "recharts";

interface DatedJitSuite {
  readonly date: string;
  readonly suite: JitBenchSuite;
}

/** A single point in the JIT speedup trend. */
interface SpeedupTrendPoint {
  readonly date: string;
  readonly [scenario: string]: string | number | null;
}

export function CompareView() {
  const [index, setIndex] = useState<DashboardIndex | null>(null);
  const [jitSuites, setJitSuites] = useState<readonly DatedJitSuite[]>([]);
  const [crossClient, setCrossClient] = useState<CrossClientSuite | null>(null);
  const [showJit, setShowJit] = useState(true);
  const [range, setRange] = useState<DateRange>("30d");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const idx = await fetchIndex(DATA_BASE_URL);
        if (cancelled) return;
        setIndex(idx);

        const loaded: DatedJitSuite[] = [];
        for (const run of idx.runs) {
          if (!run.jit_bench) continue;
          const suite = await fetchJitBenchSuite(DATA_BASE_URL, run.jit_bench);
          if (cancelled) return;
          loaded.push({ date: run.date, suite });
        }
        setJitSuites(loaded);

        const latest = idx.runs[idx.runs.length - 1];
        if (latest?.cross_client) {
          const cc = await fetchCrossClientSuite(DATA_BASE_URL, latest.cross_client);
          if (!cancelled) setCrossClient(cc);
        }
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : "Unknown error");
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    load();
    return () => { cancelled = true; };
  }, []);

  const filteredJitSuites = useMemo(() => {
    if (range === "All") return jitSuites;
    const days = range === "7d" ? 7 : 30;
    const cutoff = new Date();
    cutoff.setDate(cutoff.getDate() - days);
    const cutoffStr = cutoff.toISOString().slice(0, 10);
    return jitSuites.filter((s) => s.date >= cutoffStr);
  }, [jitSuites, range]);

  const jitScenarios = useMemo(() => {
    const set = new Set<string>();
    for (const { suite } of jitSuites) {
      for (const r of suite.results) {
        if (r.speedup !== null) set.add(r.scenario);
      }
    }
    return [...set];
  }, [jitSuites]);

  const trendData = useMemo((): readonly SpeedupTrendPoint[] => {
    return filteredJitSuites.map(({ date, suite }) => {
      const point: Record<string, string | number | null> = { date };
      for (const sc of jitScenarios) {
        const result = suite.results.find((r) => r.scenario === sc);
        point[sc] = result?.speedup ?? null;
      }
      return point as SpeedupTrendPoint;
    });
  }, [filteredJitSuites, jitScenarios]);

  const latestJitSuite = jitSuites.length > 0 ? jitSuites[jitSuites.length - 1].suite : null;

  const SCENARIO_COLORS = [COLORS.jit, COLORS.geth, COLORS.reth, COLORS.interpreter];

  if (error) {
    return <p className="text-tokamak-red">Error: {error}</p>;
  }

  if (loading) {
    return <p className="text-slate-400">Loading comparison data...</p>;
  }

  return (
    <div className="space-y-8">
      <div className="flex flex-wrap items-center gap-4">
        <JitToggle enabled={showJit} onToggle={setShowJit} />
        <DateRangePicker selected={range} onSelect={setRange} />
      </div>

      {showJit && latestJitSuite && (
        <section className="space-y-4">
          <h2 className="text-xl font-semibold">JIT vs Interpreter</h2>

          {trendData.length > 1 && (
            <div className="rounded-lg border border-tokamak-border bg-tokamak-card p-4">
              <h3 className="mb-3 text-sm font-medium text-slate-400">Speedup Trend</h3>
              <ResponsiveContainer width="100%" height={300}>
                <LineChart data={trendData as SpeedupTrendPoint[]}>
                  <CartesianGrid strokeDasharray="3 3" stroke={COLORS.grid} />
                  <XAxis dataKey="date" stroke={COLORS.text} tick={{ fontSize: 12 }} />
                  <YAxis
                    stroke={COLORS.text}
                    tick={{ fontSize: 12 }}
                    tickFormatter={(v: number) => formatSpeedup(v)}
                  />
                  <Tooltip
                    contentStyle={{ backgroundColor: "#1a1d2e", border: "1px solid #2a2d3e" }}
                    labelStyle={{ color: "#fff" }}
                    formatter={(value: number) => [formatSpeedup(value), undefined] as const}
                  />
                  <Legend />
                  {jitScenarios.map((sc, i) => (
                    <Line
                      key={sc}
                      type="monotone"
                      dataKey={sc}
                      stroke={SCENARIO_COLORS[i % SCENARIO_COLORS.length]}
                      strokeWidth={2}
                      dot={{ r: 3 }}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}

          <div className="rounded-lg border border-tokamak-border bg-tokamak-card">
            <JitSpeedupTable results={latestJitSuite.results} />
          </div>
        </section>
      )}

      {crossClient && (
        <section className="space-y-4">
          <h2 className="text-xl font-semibold">Cross-Client Comparison</h2>
          <p className="text-sm text-slate-400">
            ethrex as baseline (1.00x). Higher ratio = slower than ethrex.
          </p>
          <div className="rounded-lg border border-tokamak-border bg-tokamak-card">
            <CrossClientTable scenarios={crossClient.scenarios} />
          </div>
        </section>
      )}

      {!latestJitSuite && !crossClient && (
        <p className="text-slate-400">No comparison data available.</p>
      )}
    </div>
  );
}

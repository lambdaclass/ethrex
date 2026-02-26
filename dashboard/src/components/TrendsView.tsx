import { useEffect, useState, useMemo } from "react";
import { fetchIndex, fetchBenchSuite, buildTrendData } from "@/lib/data";
import { DATA_BASE_URL } from "@/lib/constants";
import { TrendChart } from "./TrendChart";
import { ScenarioSelector } from "./ScenarioSelector";
import { DateRangePicker, type DateRange } from "./DateRangePicker";
import type { BenchSuite, DashboardIndex } from "@/types";

interface DatedSuite {
  readonly date: string;
  readonly suite: BenchSuite;
}

export function TrendsView() {
  const [index, setIndex] = useState<DashboardIndex | null>(null);
  const [suites, setSuites] = useState<readonly DatedSuite[]>([]);
  const [scenario, setScenario] = useState<string>("");
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

        const loaded: DatedSuite[] = [];
        for (const run of idx.runs) {
          const suite = await fetchBenchSuite(DATA_BASE_URL, run.bench);
          if (cancelled) return;
          loaded.push({ date: run.date, suite });
        }
        setSuites(loaded);

        if (loaded.length > 0 && loaded[0].suite.results.length > 0) {
          setScenario(loaded[0].suite.results[0].scenario);
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

  const scenarios = useMemo(() => {
    const set = new Set<string>();
    for (const { suite } of suites) {
      for (const r of suite.results) {
        set.add(r.scenario);
      }
    }
    return [...set];
  }, [suites]);

  const filteredSuites = useMemo(() => {
    if (range === "All") return suites;
    const days = range === "7d" ? 7 : 30;
    const cutoff = new Date();
    cutoff.setDate(cutoff.getDate() - days);
    const cutoffStr = cutoff.toISOString().slice(0, 10);
    return suites.filter((s) => s.date >= cutoffStr);
  }, [suites, range]);

  const trendData = useMemo(
    () => buildTrendData(filteredSuites, scenario),
    [filteredSuites, scenario],
  );

  if (error) {
    return <p className="text-tokamak-red">Error: {error}</p>;
  }

  if (loading) {
    return <p className="text-slate-400">Loading trends...</p>;
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-4">
        {scenarios.length > 0 && (
          <ScenarioSelector scenarios={scenarios} selected={scenario} onSelect={setScenario} />
        )}
        <DateRangePicker selected={range} onSelect={setRange} />
      </div>
      <div className="rounded-lg border border-tokamak-border bg-tokamak-card p-4">
        <TrendChart data={trendData} />
      </div>
    </div>
  );
}

import { DashboardIndexSchema, BenchSuiteSchema, JitBenchSuiteSchema, CrossClientSuiteSchema } from "@/types/schemas";
import type { BenchSuite, CrossClientSuite, DashboardIndex, JitBenchSuite } from "@/types";

/** Validate that a relative path stays within bounds (no traversal). */
function validatePath(path: string): void {
  if (path.startsWith("/") || path.includes("..")) {
    throw new Error(`Invalid path: traversal not allowed: ${path}`);
  }
}

async function fetchJson(url: string): Promise<unknown> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`Failed to fetch: ${res.status} ${res.statusText}`);
  }
  return res.json();
}

/** Fetch and validate the dashboard index manifest. */
export async function fetchIndex(baseUrl: string): Promise<DashboardIndex> {
  const data = await fetchJson(`${baseUrl}/index.json`);
  return DashboardIndexSchema.parse(data);
}

/** Fetch and validate a benchmark suite JSON file. */
export async function fetchBenchSuite(baseUrl: string, path: string): Promise<BenchSuite> {
  validatePath(path);
  const data = await fetchJson(`${baseUrl}/${path}`);
  return BenchSuiteSchema.parse(data);
}

/** Fetch and validate a JIT benchmark suite JSON file. */
export async function fetchJitBenchSuite(baseUrl: string, path: string): Promise<JitBenchSuite> {
  validatePath(path);
  const data = await fetchJson(`${baseUrl}/${path}`);
  return JitBenchSuiteSchema.parse(data);
}

/** Fetch and validate a cross-client benchmark suite JSON file. */
export async function fetchCrossClientSuite(baseUrl: string, path: string): Promise<CrossClientSuite> {
  validatePath(path);
  const data = await fetchJson(`${baseUrl}/${path}`);
  return CrossClientSuiteSchema.parse(data);
}

/** A single data point in a trend time series. */
export interface TrendPoint {
  readonly date: string;
  readonly commit: string;
  readonly mean_ns: number;
  readonly ci_lower_ns?: number;
  readonly ci_upper_ns?: number;
}

/** Build a trend time series for a specific scenario from multiple dated suites. */
export function buildTrendData(
  suites: ReadonlyArray<{ readonly date: string; readonly suite: BenchSuite }>,
  scenario: string,
): readonly TrendPoint[] {
  return suites.flatMap(({ date, suite }) => {
    const result = suite.results.find((r) => r.scenario === scenario);
    if (!result) return [];

    const mean_ns = result.stats?.mean_ns ?? result.total_duration_ns / result.runs;

    return [{
      date,
      commit: suite.commit,
      mean_ns,
      ci_lower_ns: result.stats?.ci_lower_ns,
      ci_upper_ns: result.stats?.ci_upper_ns,
    }];
  });
}

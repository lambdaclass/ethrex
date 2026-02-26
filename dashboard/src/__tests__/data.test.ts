import { describe, it, expect, vi, beforeEach } from "vitest";
import { fetchIndex, fetchBenchSuite, fetchJitBenchSuite, fetchCrossClientSuite, buildTrendData } from "@/lib/data";
import type { BenchSuite, DashboardIndex, JitBenchSuite } from "@/types";

import indexFixture from "../../fixtures/index.json";
import benchFixture from "../../fixtures/2026-02-26/68a325fcf-bench.json";
import jitBenchFixture from "../../fixtures/2026-02-26/68a325fcf-jit-bench.json";
import crossClientFixture from "../../fixtures/2026-02-26/68a325fcf-cross-client.json";

const mockFetch = vi.fn();

beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
});

function mockJsonResponse(data: unknown) {
  return { ok: true, json: () => Promise.resolve(data) };
}

function mockErrorResponse() {
  return { ok: false, status: 404, statusText: "Not Found" };
}

describe("fetchIndex", () => {
  it("fetches and validates index", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(indexFixture));
    const result = await fetchIndex("http://localhost/data");
    expect(result.runs).toHaveLength(7);
    expect(result.runs[6].commit).toBe("68a325fcf");
    expect(mockFetch).toHaveBeenCalledWith("http://localhost/data/index.json");
  });

  it("throws on fetch error", async () => {
    mockFetch.mockResolvedValueOnce(mockErrorResponse());
    await expect(fetchIndex("http://localhost/data")).rejects.toThrow("Failed to fetch");
  });

  it("throws on invalid schema", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse({ invalid: true }));
    await expect(fetchIndex("http://localhost/data")).rejects.toThrow();
  });
});

describe("fetchBenchSuite", () => {
  it("fetches and validates bench suite", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(benchFixture));
    const result = await fetchBenchSuite("http://localhost/data", "2026-02-26/68a325fcf-bench.json");
    expect(result.commit).toBe("68a325fcf");
    expect(result.results.length).toBeGreaterThan(0);
  });

  it("preserves stats when present", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(benchFixture));
    const result = await fetchBenchSuite("http://localhost/data", "2026-02-26/68a325fcf-bench.json");
    expect(result.results[0].stats).toBeDefined();
    expect(result.results[0].stats?.samples).toBe(10);
  });

  it("throws on network error", async () => {
    mockFetch.mockRejectedValueOnce(new Error("Network error"));
    await expect(
      fetchBenchSuite("http://localhost/data", "path.json")
    ).rejects.toThrow("Network error");
  });

  it("rejects path traversal with ..", async () => {
    await expect(
      fetchBenchSuite("http://localhost/data", "../etc/passwd")
    ).rejects.toThrow("traversal not allowed");
  });

  it("rejects absolute paths", async () => {
    await expect(
      fetchBenchSuite("http://localhost/data", "/etc/passwd")
    ).rejects.toThrow("traversal not allowed");
  });
});

describe("fetchJitBenchSuite", () => {
  it("fetches and validates jit bench suite", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(jitBenchFixture));
    const result = await fetchJitBenchSuite("http://localhost/data", "path.json");
    expect(result.results.length).toBeGreaterThan(0);
    expect(result.results[0].speedup).toBe(2.76);
  });
});

describe("fetchCrossClientSuite", () => {
  it("fetches and validates cross-client suite", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(crossClientFixture));
    const result = await fetchCrossClientSuite("http://localhost/data", "path.json");
    expect(result.scenarios).toHaveLength(4);
    expect(result.scenarios[0].scenario).toBe("Fibonacci");
    expect(result.scenarios[0].results).toHaveLength(3);
  });

  it("validates client names in results", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(crossClientFixture));
    const result = await fetchCrossClientSuite("http://localhost/data", "path.json");
    const clients = result.scenarios[0].results.map((r) => r.client_name);
    expect(clients).toContain("ethrex");
    expect(clients).toContain("geth");
    expect(clients).toContain("reth");
  });

  it("rejects path traversal", async () => {
    await expect(
      fetchCrossClientSuite("http://localhost/data", "../secret.json")
    ).rejects.toThrow("traversal not allowed");
  });

  it("includes ethrex_mean_ns per scenario", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonResponse(crossClientFixture));
    const result = await fetchCrossClientSuite("http://localhost/data", "path.json");
    for (const sc of result.scenarios) {
      expect(sc.ethrex_mean_ns).toBeGreaterThan(0);
    }
  });
});

describe("buildTrendData", () => {
  it("builds trend series from multiple suites", () => {
    const suites: ReadonlyArray<{ readonly date: string; readonly suite: BenchSuite }> = [
      {
        date: "2026-02-25",
        suite: {
          timestamp: "1740470400", commit: "aaa",
          results: [{ scenario: "Fibonacci", total_duration_ns: 6000000000, runs: 10, opcode_timings: [] }],
        },
      },
      {
        date: "2026-02-26",
        suite: {
          timestamp: "1740556800", commit: "bbb",
          results: [{ scenario: "Fibonacci", total_duration_ns: 5000000000, runs: 10, opcode_timings: [] }],
        },
      },
    ];

    const trend = buildTrendData(suites, "Fibonacci");
    expect(trend).toHaveLength(2);
    expect(trend[0].date).toBe("2026-02-25");
    expect(trend[0].mean_ns).toBe(600000000);
    expect(trend[1].mean_ns).toBe(500000000);
  });

  it("uses stats.mean_ns when available", () => {
    const suites: ReadonlyArray<{ readonly date: string; readonly suite: BenchSuite }> = [
      {
        date: "2026-02-26",
        suite: {
          timestamp: "1740556800", commit: "bbb",
          results: [{
            scenario: "Fibonacci", total_duration_ns: 5000000000, runs: 10, opcode_timings: [],
            stats: {
              mean_ns: 490000000, stddev_ns: 25000000,
              ci_lower_ns: 474000000, ci_upper_ns: 506000000,
              min_ns: 460000000, max_ns: 520000000, samples: 10,
            },
          }],
        },
      },
    ];

    const trend = buildTrendData(suites, "Fibonacci");
    expect(trend[0].mean_ns).toBe(490000000);
    expect(trend[0].ci_lower_ns).toBe(474000000);
    expect(trend[0].ci_upper_ns).toBe(506000000);
  });

  it("returns empty for unknown scenario", () => {
    const suites: ReadonlyArray<{ readonly date: string; readonly suite: BenchSuite }> = [
      {
        date: "2026-02-26",
        suite: {
          timestamp: "1740556800", commit: "bbb",
          results: [{ scenario: "Fibonacci", total_duration_ns: 5000000000, runs: 10, opcode_timings: [] }],
        },
      },
    ];
    const trend = buildTrendData(suites, "Unknown");
    expect(trend).toHaveLength(0);
  });

  it("handles empty suites array", () => {
    const trend = buildTrendData([], "Fibonacci");
    expect(trend).toHaveLength(0);
  });
});

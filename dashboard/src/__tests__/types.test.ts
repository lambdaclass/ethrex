import { describe, it, expect } from "vitest";
import {
  BenchStatsSchema,
  OpcodeEntrySchema,
  BenchResultSchema,
  BenchSuiteSchema,
  RegressionSchema,
  RegressionStatusSchema,
  ThresholdsSchema,
  RegressionReportSchema,
  JitBenchResultSchema,
  JitBenchSuiteSchema,
  JitSpeedupDeltaSchema,
  JitRegressionReportSchema,
  CrossClientResultSchema,
  CrossClientScenarioSchema,
  CrossClientSuiteSchema,
  DashboardIndexSchema,
} from "@/types/schemas";

describe("BenchStats schema", () => {
  it("parses valid stats", () => {
    const data = {
      mean_ns: 100000000.0,
      stddev_ns: 5000000.0,
      ci_lower_ns: 96040000.0,
      ci_upper_ns: 103960000.0,
      min_ns: 95000000,
      max_ns: 108000000,
      samples: 10,
    };
    const result = BenchStatsSchema.parse(data);
    expect(result.mean_ns).toBe(100000000.0);
    expect(result.samples).toBe(10);
  });

  it("rejects missing fields", () => {
    expect(() => BenchStatsSchema.parse({ mean_ns: 1 })).toThrow();
  });
});

describe("OpcodeEntry schema", () => {
  it("parses valid entry", () => {
    const data = { opcode: "ADD", avg_ns: 150, total_ns: 1500, count: 10 };
    const result = OpcodeEntrySchema.parse(data);
    expect(result.opcode).toBe("ADD");
    expect(result.count).toBe(10);
  });

  it("rejects negative count", () => {
    expect(() =>
      OpcodeEntrySchema.parse({ opcode: "ADD", avg_ns: 1, total_ns: 1, count: -1 })
    ).toThrow();
  });
});

describe("BenchResult schema", () => {
  it("parses result without stats", () => {
    const data = {
      scenario: "Fibonacci",
      total_duration_ns: 5000000,
      runs: 10,
      opcode_timings: [{ opcode: "ADD", avg_ns: 100, total_ns: 1000, count: 10 }],
    };
    const result = BenchResultSchema.parse(data);
    expect(result.scenario).toBe("Fibonacci");
    expect(result.stats).toBeUndefined();
  });

  it("parses result with stats", () => {
    const data = {
      scenario: "Fibonacci",
      total_duration_ns: 5000000,
      runs: 10,
      opcode_timings: [],
      stats: {
        mean_ns: 500000, stddev_ns: 1000, ci_lower_ns: 498000,
        ci_upper_ns: 502000, min_ns: 490000, max_ns: 510000, samples: 10,
      },
    };
    const result = BenchResultSchema.parse(data);
    expect(result.stats).toBeDefined();
    expect(result.stats?.samples).toBe(10);
  });
});

describe("BenchSuite schema", () => {
  it("parses valid suite", () => {
    const data = {
      timestamp: "1700000000",
      commit: "abc123def",
      results: [{
        scenario: "Fibonacci",
        total_duration_ns: 5000000,
        runs: 10,
        opcode_timings: [],
      }],
    };
    const result = BenchSuiteSchema.parse(data);
    expect(result.commit).toBe("abc123def");
    expect(result.results).toHaveLength(1);
  });
});

describe("RegressionStatus schema", () => {
  it("accepts valid statuses", () => {
    expect(RegressionStatusSchema.parse("Stable")).toBe("Stable");
    expect(RegressionStatusSchema.parse("Warning")).toBe("Warning");
    expect(RegressionStatusSchema.parse("Regression")).toBe("Regression");
  });

  it("rejects invalid status", () => {
    expect(() => RegressionStatusSchema.parse("Unknown")).toThrow();
  });
});

describe("RegressionReport schema", () => {
  it("parses valid report", () => {
    const data = {
      status: "Stable",
      thresholds: { warning_percent: 20.0, regression_percent: 50.0 },
      regressions: [],
      improvements: [],
    };
    const result = RegressionReportSchema.parse(data);
    expect(result.status).toBe("Stable");
  });

  it("parses report with entries", () => {
    const data = {
      status: "Regression",
      thresholds: { warning_percent: 20.0, regression_percent: 50.0 },
      regressions: [{
        scenario: "Fibonacci", opcode: "ADD",
        baseline_avg_ns: 100, current_avg_ns: 200, change_percent: 100.0,
      }],
      improvements: [],
    };
    const result = RegressionReportSchema.parse(data);
    expect(result.regressions).toHaveLength(1);
  });
});

describe("JitBenchResult schema", () => {
  it("parses result with JIT available", () => {
    const data = {
      scenario: "Fibonacci",
      interpreter_ns: 5000000,
      jit_ns: 2000000,
      speedup: 2.5,
      runs: 10,
    };
    const result = JitBenchResultSchema.parse(data);
    expect(result.speedup).toBe(2.5);
  });

  it("parses result without JIT", () => {
    const data = {
      scenario: "Fibonacci",
      interpreter_ns: 5000000,
      jit_ns: null,
      speedup: null,
      runs: 10,
    };
    const result = JitBenchResultSchema.parse(data);
    expect(result.jit_ns).toBeNull();
    expect(result.speedup).toBeNull();
  });
});

describe("JitBenchSuite schema", () => {
  it("parses valid suite", () => {
    const data = {
      timestamp: "1700000000",
      commit: "abc123d",
      results: [{
        scenario: "Fibonacci",
        interpreter_ns: 5000000,
        jit_ns: 2000000,
        speedup: 2.5,
        runs: 10,
      }],
    };
    const result = JitBenchSuiteSchema.parse(data);
    expect(result.results).toHaveLength(1);
  });
});

describe("JitRegressionReport schema", () => {
  it("parses valid report", () => {
    const data = {
      status: "Stable",
      threshold_percent: 20.0,
      regressions: [],
      improvements: [{
        scenario: "Fibonacci",
        baseline_speedup: 2.0,
        current_speedup: 2.5,
        change_percent: 25.0,
      }],
    };
    const result = JitRegressionReportSchema.parse(data);
    expect(result.improvements).toHaveLength(1);
  });
});

describe("CrossClientResult schema", () => {
  it("parses without stats", () => {
    const data = { client_name: "geth", scenario: "Fibonacci", mean_ns: 1500000.0 };
    const result = CrossClientResultSchema.parse(data);
    expect(result.client_name).toBe("geth");
    expect(result.stats).toBeUndefined();
  });

  it("parses with stats", () => {
    const data = {
      client_name: "reth", scenario: "Fibonacci", mean_ns: 3000000.0,
      stats: {
        mean_ns: 3000000.0, stddev_ns: 100000.0, ci_lower_ns: 2900000.0,
        ci_upper_ns: 3100000.0, min_ns: 2800000, max_ns: 3200000, samples: 10,
      },
    };
    const result = CrossClientResultSchema.parse(data);
    expect(result.stats?.samples).toBe(10);
  });
});

describe("CrossClientSuite schema", () => {
  it("parses valid suite", () => {
    const data = {
      timestamp: "1700000000",
      commit: "abc123d",
      scenarios: [{
        scenario: "Fibonacci",
        ethrex_mean_ns: 1000000.0,
        results: [{ client_name: "ethrex", scenario: "Fibonacci", mean_ns: 1000000.0 }],
      }],
    };
    const result = CrossClientSuiteSchema.parse(data);
    expect(result.scenarios).toHaveLength(1);
  });
});

describe("DashboardIndex schema", () => {
  it("parses valid index", () => {
    const data = {
      runs: [{
        date: "2026-02-26",
        commit: "abc123def",
        bench: "2026-02-26/abc123def-bench.json",
        jit_bench: "2026-02-26/abc123def-jit-bench.json",
        regression: "2026-02-26/abc123def-regression.json",
      }],
    };
    const result = DashboardIndexSchema.parse(data);
    expect(result.runs).toHaveLength(1);
    expect(result.runs[0].date).toBe("2026-02-26");
  });

  it("accepts runs with optional fields", () => {
    const data = {
      runs: [{
        date: "2026-02-26",
        commit: "abc123def",
        bench: "2026-02-26/abc123def-bench.json",
      }],
    };
    const result = DashboardIndexSchema.parse(data);
    expect(result.runs[0].jit_bench).toBeUndefined();
    expect(result.runs[0].regression).toBeUndefined();
  });
});

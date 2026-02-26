/** Statistical summary of benchmark run durations. */
export interface BenchStats {
  readonly mean_ns: number;
  readonly stddev_ns: number;
  readonly ci_lower_ns: number;
  readonly ci_upper_ns: number;
  readonly min_ns: number;
  readonly max_ns: number;
  readonly samples: number;
}

/** Per-opcode timing data. */
export interface OpcodeEntry {
  readonly opcode: string;
  readonly avg_ns: number;
  readonly total_ns: number;
  readonly count: number;
}

/** Single scenario benchmark result. */
export interface BenchResult {
  readonly scenario: string;
  readonly total_duration_ns: number;
  readonly runs: number;
  readonly opcode_timings: readonly OpcodeEntry[];
  readonly stats?: BenchStats;
}

/** Full benchmark suite with metadata. */
export interface BenchSuite {
  readonly timestamp: string;
  readonly commit: string;
  readonly results: readonly BenchResult[];
}

/** Regression status enum. */
export type RegressionStatus = "Stable" | "Warning" | "Regression";

/** A single opcode-level regression entry. */
export interface Regression {
  readonly scenario: string;
  readonly opcode: string;
  readonly baseline_avg_ns: number;
  readonly current_avg_ns: number;
  readonly change_percent: number;
}

/** Configurable regression detection thresholds. */
export interface Thresholds {
  readonly warning_percent: number;
  readonly regression_percent: number;
}

/** Regression report comparing baseline vs current. */
export interface RegressionReport {
  readonly status: RegressionStatus;
  readonly thresholds: Thresholds;
  readonly regressions: readonly Regression[];
  readonly improvements: readonly Regression[];
}

/** JIT vs interpreter benchmark result for a single scenario. */
export interface JitBenchResult {
  readonly scenario: string;
  readonly interpreter_ns: number;
  readonly jit_ns: number | null;
  readonly speedup: number | null;
  readonly runs: number;
  readonly interp_stats?: BenchStats;
  readonly jit_stats?: BenchStats;
}

/** Full JIT benchmark suite with metadata. */
export interface JitBenchSuite {
  readonly timestamp: string;
  readonly commit: string;
  readonly results: readonly JitBenchResult[];
}

/** JIT speedup regression entry. */
export interface JitSpeedupDelta {
  readonly scenario: string;
  readonly baseline_speedup: number;
  readonly current_speedup: number;
  readonly change_percent: number;
}

/** JIT speedup regression report. */
export interface JitRegressionReport {
  readonly status: RegressionStatus;
  readonly threshold_percent: number;
  readonly regressions: readonly JitSpeedupDelta[];
  readonly improvements: readonly JitSpeedupDelta[];
}

/** Cross-client result for a single client on a single scenario. */
export interface CrossClientResult {
  readonly client_name: string;
  readonly scenario: string;
  readonly mean_ns: number;
  readonly stats?: BenchStats;
}

/** Aggregated results for a single scenario across clients. */
export interface CrossClientScenario {
  readonly scenario: string;
  readonly results: readonly CrossClientResult[];
  readonly ethrex_mean_ns: number;
}

/** Full cross-client benchmark suite. */
export interface CrossClientSuite {
  readonly timestamp: string;
  readonly commit: string;
  readonly scenarios: readonly CrossClientScenario[];
}

/** A single run entry in the dashboard index. */
export interface IndexEntry {
  readonly date: string;
  readonly commit: string;
  readonly bench: string;
  readonly jit_bench?: string;
  readonly regression?: string;
  readonly jit_regression?: string;
  readonly cross_client?: string;
}

/** Dashboard index manifest listing all available runs. */
export interface DashboardIndex {
  readonly runs: readonly IndexEntry[];
}

import { z } from "zod";

export const BenchStatsSchema = z.object({
  mean_ns: z.number(),
  stddev_ns: z.number(),
  ci_lower_ns: z.number(),
  ci_upper_ns: z.number(),
  min_ns: z.number(),
  max_ns: z.number(),
  samples: z.number().int().nonnegative(),
});

export const OpcodeEntrySchema = z.object({
  opcode: z.string(),
  avg_ns: z.number().nonnegative(),
  total_ns: z.number().nonnegative(),
  count: z.number().int().nonnegative(),
});

export const BenchResultSchema = z.object({
  scenario: z.string(),
  total_duration_ns: z.number().nonnegative(),
  runs: z.number().int().nonnegative(),
  opcode_timings: z.array(OpcodeEntrySchema),
  stats: BenchStatsSchema.optional(),
});

const TimestampString = z.string().regex(/^\d+$/, "Must be a numeric Unix timestamp");
const CommitHash = z.string().regex(/^[a-f0-9]{7,40}$/, "Must be a valid git commit hash");

export const BenchSuiteSchema = z.object({
  timestamp: TimestampString,
  commit: CommitHash,
  results: z.array(BenchResultSchema),
});

export const RegressionStatusSchema = z.enum(["Stable", "Warning", "Regression"]);

export const RegressionSchema = z.object({
  scenario: z.string(),
  opcode: z.string(),
  baseline_avg_ns: z.number(),
  current_avg_ns: z.number(),
  change_percent: z.number(),
});

export const ThresholdsSchema = z.object({
  warning_percent: z.number(),
  regression_percent: z.number(),
});

export const RegressionReportSchema = z.object({
  status: RegressionStatusSchema,
  thresholds: ThresholdsSchema,
  regressions: z.array(RegressionSchema),
  improvements: z.array(RegressionSchema),
});

export const JitBenchResultSchema = z.object({
  scenario: z.string(),
  interpreter_ns: z.number().nonnegative(),
  jit_ns: z.number().nonnegative().nullable(),
  speedup: z.number().nullable(),
  runs: z.number().int().nonnegative(),
  interp_stats: BenchStatsSchema.optional(),
  jit_stats: BenchStatsSchema.optional(),
});

export const JitBenchSuiteSchema = z.object({
  timestamp: TimestampString,
  commit: CommitHash,
  results: z.array(JitBenchResultSchema),
});

export const JitSpeedupDeltaSchema = z.object({
  scenario: z.string(),
  baseline_speedup: z.number(),
  current_speedup: z.number(),
  change_percent: z.number(),
});

export const JitRegressionReportSchema = z.object({
  status: RegressionStatusSchema,
  threshold_percent: z.number(),
  regressions: z.array(JitSpeedupDeltaSchema),
  improvements: z.array(JitSpeedupDeltaSchema),
});

export const CrossClientResultSchema = z.object({
  client_name: z.string(),
  scenario: z.string(),
  mean_ns: z.number(),
  stats: BenchStatsSchema.optional(),
});

export const CrossClientScenarioSchema = z.object({
  scenario: z.string(),
  results: z.array(CrossClientResultSchema),
  ethrex_mean_ns: z.number(),
});

export const CrossClientSuiteSchema = z.object({
  timestamp: TimestampString,
  commit: CommitHash,
  scenarios: z.array(CrossClientScenarioSchema),
});

const DateString = z.string().regex(/^\d{4}-\d{2}-\d{2}$/, "Must be YYYY-MM-DD");
const RelativePath = z.string().regex(/^[a-zA-Z0-9._/-]+$/, "Must be a safe relative path");

export const IndexEntrySchema = z.object({
  date: DateString,
  commit: CommitHash,
  bench: RelativePath,
  jit_bench: RelativePath.optional(),
  regression: RelativePath.optional(),
  jit_regression: RelativePath.optional(),
  cross_client: RelativePath.optional(),
});

export const DashboardIndexSchema = z.object({
  runs: z.array(IndexEntrySchema),
});

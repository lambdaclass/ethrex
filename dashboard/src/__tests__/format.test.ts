import { describe, it, expect } from "vitest";
import { formatNs, formatSpeedup, formatPercent, formatCommit } from "@/lib/format";

describe("formatNs", () => {
  it("formats nanoseconds as ns", () => {
    expect(formatNs(500)).toBe("500.0 ns");
  });

  it("formats microseconds", () => {
    expect(formatNs(1_500)).toBe("1.50 \u00b5s");
  });

  it("formats milliseconds", () => {
    expect(formatNs(1_500_000)).toBe("1.50 ms");
  });

  it("formats seconds", () => {
    expect(formatNs(1_500_000_000)).toBe("1.50 s");
  });

  it("handles zero", () => {
    expect(formatNs(0)).toBe("0.0 ns");
  });

  it("handles very large values", () => {
    expect(formatNs(60_000_000_000)).toBe("60.0 s");
  });
});

describe("formatSpeedup", () => {
  it("formats positive speedup", () => {
    expect(formatSpeedup(2.5)).toBe("2.50x");
  });

  it("formats 1x speedup", () => {
    expect(formatSpeedup(1.0)).toBe("1.00x");
  });

  it("returns N/A for null", () => {
    expect(formatSpeedup(null)).toBe("N/A");
  });

  it("formats fractional speedup (slowdown)", () => {
    expect(formatSpeedup(0.5)).toBe("0.50x");
  });
});

describe("formatPercent", () => {
  it("formats positive change with + sign", () => {
    expect(formatPercent(25.0)).toBe("+25.0%");
  });

  it("formats negative change with - sign", () => {
    expect(formatPercent(-10.5)).toBe("-10.5%");
  });

  it("formats zero", () => {
    expect(formatPercent(0)).toBe("+0.0%");
  });
});

describe("formatCommit", () => {
  it("truncates to 7 chars", () => {
    expect(formatCommit("abc123def456789")).toBe("abc123d");
  });

  it("handles short commit", () => {
    expect(formatCommit("abc")).toBe("abc");
  });
});

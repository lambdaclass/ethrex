import { describe, it, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { StatusBadge } from "@/components/StatusBadge";
import { MetricCard } from "@/components/MetricCard";
import { BenchTable } from "@/components/BenchTable";
import { ScenarioSelector } from "@/components/ScenarioSelector";
import { DateRangePicker, type DateRange } from "@/components/DateRangePicker";
import { JitToggle } from "@/components/JitToggle";
import type { BenchResult, RegressionStatus } from "@/types";

afterEach(cleanup);

describe("StatusBadge", () => {
  it("renders Stable with green styling", () => {
    render(<StatusBadge status="Stable" />);
    const badge = screen.getByText("Stable");
    expect(badge).toBeInTheDocument();
  });

  it("renders Warning", () => {
    render(<StatusBadge status="Warning" />);
    expect(screen.getByText("Warning")).toBeInTheDocument();
  });

  it("renders Regression", () => {
    render(<StatusBadge status="Regression" />);
    expect(screen.getByText("Regression")).toBeInTheDocument();
  });
});

describe("MetricCard", () => {
  it("renders value and label", () => {
    render(<MetricCard label="Mean Time" value="500 ms" />);
    expect(screen.getByText("Mean Time")).toBeInTheDocument();
    expect(screen.getByText("500 ms")).toBeInTheDocument();
  });

  it("renders with status badge", () => {
    render(<MetricCard label="Regression Status" value="All Clear" status="Stable" />);
    expect(screen.getByText("All Clear")).toBeInTheDocument();
    expect(screen.getByText("Stable")).toBeInTheDocument();
  });
});

describe("BenchTable", () => {
  const results: BenchResult[] = [
    {
      scenario: "Fibonacci",
      total_duration_ns: 5000000000,
      runs: 10,
      opcode_timings: [
        { opcode: "ADD", avg_ns: 150, total_ns: 15000, count: 100 },
      ],
      stats: {
        mean_ns: 500000000, stddev_ns: 25000000,
        ci_lower_ns: 484510000, ci_upper_ns: 515490000,
        min_ns: 460000000, max_ns: 540000000, samples: 10,
      },
    },
    {
      scenario: "BubbleSort",
      total_duration_ns: 8000000000,
      runs: 10,
      opcode_timings: [],
    },
  ];

  it("renders scenario names", () => {
    render(<BenchTable results={results} />);
    expect(screen.getByText("Fibonacci")).toBeInTheDocument();
    expect(screen.getByText("BubbleSort")).toBeInTheDocument();
  });

  it("renders column headers", () => {
    render(<BenchTable results={results} />);
    expect(screen.getByText("Scenario")).toBeInTheDocument();
    expect(screen.getByText("Mean")).toBeInTheDocument();
    expect(screen.getByText("Runs")).toBeInTheDocument();
  });

  it("renders formatted mean time", () => {
    render(<BenchTable results={results} />);
    expect(screen.getByText("500.00 ms")).toBeInTheDocument();
  });
});

describe("ScenarioSelector", () => {
  const scenarios = ["Fibonacci", "BubbleSort", "ERC20Transfer"];

  it("renders all options", () => {
    render(<ScenarioSelector scenarios={scenarios} selected="Fibonacci" onSelect={() => {}} />);
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(3);
  });

  it("calls onSelect when changed", () => {
    let selected = "Fibonacci";
    render(
      <ScenarioSelector
        scenarios={scenarios}
        selected={selected}
        onSelect={(s) => { selected = s; }}
      />
    );
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "BubbleSort" } });
  });
});

describe("DateRangePicker", () => {
  it("renders range buttons", () => {
    render(<DateRangePicker selected="7d" onSelect={() => {}} />);
    expect(screen.getByText("7d")).toBeInTheDocument();
    expect(screen.getByText("30d")).toBeInTheDocument();
    expect(screen.getByText("All")).toBeInTheDocument();
  });

  it("calls onSelect when clicked", () => {
    let selected: DateRange = "7d";
    render(<DateRangePicker selected={selected} onSelect={(r) => { selected = r; }} />);
    fireEvent.click(screen.getByText("30d"));
  });
});

describe("JitToggle", () => {
  it("renders toggle", () => {
    render(<JitToggle enabled={true} onToggle={() => {}} />);
    expect(screen.getByText("JIT")).toBeInTheDocument();
  });

  it("calls onToggle when clicked", () => {
    let enabled = true;
    render(<JitToggle enabled={enabled} onToggle={(v) => { enabled = v; }} />);
    fireEvent.click(screen.getByRole("button"));
  });
});

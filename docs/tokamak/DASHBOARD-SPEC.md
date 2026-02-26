# F-2: Public Dashboard Design Specification

**URL**: `clients.tokamak.network`
**Status**: Design Phase
**Dependencies**: F-1 (Cross-Client Benchmarking), C-1 (JIT Benchmark CI)
**Estimated Effort**: 20-30h
**Date**: 2026-02-26

---

## 1. Overview

### Purpose

The public dashboard provides a real-time, web-accessible view of Tokamak/ethrex EVM client performance. It answers three questions that matter to node operators, L2 integrators, and the Ethereum community:

1. **How fast is ethrex compared to Geth and Reth?** (Cross-client comparison from F-1)
2. **Is ethrex getting faster or slower over time?** (Regression trends from C-1/C-3)
3. **How much does the JIT compiler help?** (JIT vs interpreter speedup from the `jit-bench` pipeline)

### Goals

- Publish every CI benchmark run automatically (zero manual intervention after setup)
- Provide historical trend lines so regressions are visible at a glance
- Show per-opcode breakdown for contributors investigating performance
- Present cross-client comparison with ethrex as the 1.00x baseline
- Surface regression alerts prominently when a merge degrades performance

### Non-Goals (v1.0)

- Interactive bytecode profiling (future debugger web UI, not this task)
- Real-time node monitoring (Grafana/Prometheus territory)
- Authenticated write access (all data is public, writes come only from CI)

---

## 2. Architecture

```
GitHub Actions CI
  |
  |  (1) Benchmark jobs produce JSON artifacts
  |      - bench-pr.json        (BenchSuite)
  |      - jit-bench-pr.json    (JitBenchSuite)
  |      - cross-client.json    (CrossClientSuite)
  |      - comparison.json      (RegressionReport)
  |      - jit-report.json      (JitRegressionReport)
  |
  v
GitHub Actions step: "Publish to Dashboard"
  |
  |  (2) POST JSON to Dashboard API (or push to data repo)
  |
  v
Data Store (GitHub Pages repo or S3 bucket)
  |
  |  (3) Static JSON files organized by date/commit
  |      data/
  |        2026-02-26/
  |          abc123-bench.json
  |          abc123-jit-bench.json
  |          abc123-cross-client.json
  |          abc123-regression.json
  |        index.json   <-- manifest of all runs
  |
  v
Static Frontend (Next.js / Astro export)
  |
  |  (4) Fetches JSON at build time (SSG) or client-side
  |      Renders charts, tables, alerts
  |
  v
clients.tokamak.network
```

### Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Data transport | Git-based data repo (GitHub Pages) | Zero infrastructure cost; CI already has write access; JSON files are version-controlled |
| Frontend framework | Static site (Next.js `output: export` or Astro) | No server needed; CDN-cacheable; cheap to host |
| Charting library | Chart.js or Recharts | Lightweight, works with SSG, good time-series support |
| API layer | None (client-side fetch from static JSON) | Eliminates backend; data is small (<1MB total); scales trivially |
| Hosting | GitHub Pages or Cloudflare Pages at `clients.tokamak.network` | Free tier sufficient; custom domain via CNAME |

---

## 3. Data Model

All data structures below are defined in the existing `tokamak-bench` crate. The dashboard consumes these JSON files directly.

### 3.1 Benchmark Suite (`BenchSuite`)

Source: `crates/tokamak-bench/src/types.rs`

```json
{
  "timestamp": "1709000000",
  "commit": "abc123def",
  "results": [
    {
      "scenario": "Fibonacci",
      "total_duration_ns": 35500000,
      "runs": 10,
      "opcode_timings": [
        {
          "opcode": "ADD",
          "avg_ns": 100,
          "total_ns": 1000,
          "count": 10
        }
      ],
      "stats": {
        "mean_ns": 3550000.0,
        "stddev_ns": 120000.0,
        "ci_lower_ns": 3475000.0,
        "ci_upper_ns": 3625000.0,
        "min_ns": 3410000,
        "max_ns": 3780000,
        "samples": 10
      }
    }
  ]
}
```

### 3.2 JIT Benchmark Suite (`JitBenchSuite`)

Source: `crates/tokamak-bench/src/types.rs`

```json
{
  "timestamp": "1709000000",
  "commit": "abc123def",
  "results": [
    {
      "scenario": "Fibonacci",
      "interpreter_ns": 3550000,
      "jit_ns": 1400000,
      "speedup": 2.53,
      "runs": 10,
      "interp_stats": {
        "mean_ns": 3550000.0,
        "stddev_ns": 120000.0,
        "ci_lower_ns": 3475000.0,
        "ci_upper_ns": 3625000.0,
        "min_ns": 3410000,
        "max_ns": 3780000,
        "samples": 10
      },
      "jit_stats": {
        "mean_ns": 1400000.0,
        "stddev_ns": 50000.0,
        "ci_lower_ns": 1369000.0,
        "ci_upper_ns": 1431000.0,
        "min_ns": 1350000,
        "max_ns": 1480000,
        "samples": 10
      }
    }
  ]
}
```

### 3.3 Cross-Client Suite (`CrossClientSuite`)

Source: `crates/tokamak-bench/src/cross_client/types.rs`

```json
{
  "timestamp": "1709000000",
  "commit": "abc123def",
  "scenarios": [
    {
      "scenario": "Fibonacci",
      "ethrex_mean_ns": 1000000.0,
      "results": [
        {
          "client_name": "ethrex",
          "scenario": "Fibonacci",
          "mean_ns": 1000000.0,
          "stats": { "...BenchStats..." }
        },
        {
          "client_name": "geth",
          "scenario": "Fibonacci",
          "mean_ns": 2500000.0,
          "stats": { "...BenchStats..." }
        },
        {
          "client_name": "reth",
          "scenario": "Fibonacci",
          "mean_ns": 1800000.0
        }
      ]
    }
  ]
}
```

### 3.4 Regression Report (`RegressionReport`)

Source: `crates/tokamak-bench/src/types.rs`

```json
{
  "status": "Stable",
  "thresholds": {
    "warning_percent": 20.0,
    "regression_percent": 50.0
  },
  "regressions": [],
  "improvements": []
}
```

### 3.5 JIT Regression Report (`JitRegressionReport`)

Source: `crates/tokamak-bench/src/types.rs`

```json
{
  "status": "Regression",
  "threshold_percent": 20.0,
  "regressions": [
    {
      "scenario": "BubbleSort",
      "baseline_speedup": 2.24,
      "current_speedup": 1.50,
      "change_percent": -33.0
    }
  ],
  "improvements": []
}
```

### 3.6 Dashboard Index Manifest (new)

A single `index.json` file at the data root that the frontend fetches to discover all available runs. This is the only new data structure the dashboard introduces.

```json
{
  "runs": [
    {
      "date": "2026-02-26",
      "commit": "abc123def",
      "branch": "feat/tokamak-proven-execution",
      "files": {
        "bench": "2026-02-26/abc123def-bench.json",
        "jit_bench": "2026-02-26/abc123def-jit-bench.json",
        "cross_client": "2026-02-26/abc123def-cross-client.json",
        "regression": "2026-02-26/abc123def-regression.json",
        "jit_regression": "2026-02-26/abc123def-jit-regression.json"
      },
      "status": "Stable"
    }
  ],
  "latest_commit": "abc123def",
  "total_runs": 42
}
```

---

## 4. Pages / Views

### 4.1 Landing Page (`/`)

**Purpose**: At-a-glance project health and headline numbers.

**Content**:
- Hero banner: "ethrex EVM Client Performance"
- Key metric cards:
  - **JIT Speedup**: Latest average JIT vs interpreter ratio (e.g., "2.53x on Fibonacci")
  - **Cross-Client**: ethrex vs Geth/Reth headline comparison (e.g., "1.4x faster than Geth on Fibonacci")
  - **Regression Status**: Badge showing Stable / Warning / Regression for the latest run
  - **Hive Pass Rate**: 6/6 suites passing (static until Hive CI exports JSON)
  - **Sync Time**: Latest Hoodi snap sync time (e.g., "1h48m")
- Latest benchmark run summary table (scenario, mean time, JIT speedup)
- Link to detailed views

### 4.2 Historical Trends (`/trends`)

**Purpose**: Show how performance changes over time across commits.

**Charts** (one per scenario):
- **X-axis**: Commit hash (abbreviated) or date
- **Y-axis**: Execution time (ms)
- **Lines**: Interpreter mean, JIT mean (where available)
- **Error bands**: 95% CI shaded region (from `BenchStats.ci_lower_ns` / `ci_upper_ns`)
- **Annotations**: Red vertical lines for commits flagged as regressions

**Controls**:
- Scenario selector dropdown (Fibonacci, BubbleSort, Factorial, ManyHashes, etc.)
- Date range picker (last 7 days, 30 days, all time)
- Toggle JIT line on/off

**Data source**: Iterate over `index.json` runs, fetch each `*-bench.json` and `*-jit-bench.json`, extract `stats.mean_ns` per scenario.

### 4.3 JIT vs Interpreter (`/jit`)

**Purpose**: Detailed JIT compilation impact analysis.

**Content**:
- Bar chart: Side-by-side interpreter vs JIT for each scenario (latest run)
- Speedup ratio badges per scenario
- Historical JIT speedup trend (line chart of `speedup` field over commits)
- Table with full statistics:

| Scenario | Interpreter (ms) | JIT (ms) | Speedup | Interp Stddev | JIT Stddev | Interp 95% CI | JIT 95% CI |
|----------|------------------|----------|---------|---------------|------------|----------------|------------|

- Notes section explaining:
  - Which scenarios are interpreter-only (bytecode > 24KB: Push, MstoreBench, SstoreBench)
  - Which scenarios are skipped (recursive CALL: FibonacciRecursive, FactorialRecursive, ERC20*)
  - Link to D-1 and D-2 decision rationale

**Data source**: `*-jit-bench.json` files.

### 4.4 Cross-Client Comparison (`/compare`)

**Purpose**: Show ethrex performance relative to other Ethereum clients.

**Content**:
- Grouped bar chart: Execution time per scenario, grouped by client (ethrex, geth, reth)
- Ratio table with ethrex as 1.00x baseline:

| Scenario | ethrex (ms) | ethrex ratio | geth (ms) | geth ratio | reth (ms) | reth ratio |
|----------|-------------|--------------|-----------|------------|-----------|------------|

- Footer note: "Ratio: relative to ethrex (1.00x = same speed, >1.00x = slower than ethrex)"
- Methodology note: ethrex runs in-process (no RPC overhead), Geth/Reth via `eth_call` with state overrides. This gives Geth/Reth a disadvantage due to RPC serialization latency -- clearly noted with a caveat.
- Historical cross-client trend (if sufficient data points)

**Data source**: `*-cross-client.json` files. The `CrossClientSuite` type from `crates/tokamak-bench/src/cross_client/types.rs` already has `ethrex_mean_ns` as baseline.

### 4.5 Per-Opcode Breakdown (`/opcodes`)

**Purpose**: Deep-dive into which EVM opcodes contribute most to execution time.

**Content**:
- Stacked bar chart: Top 10 opcodes by total time per scenario
- Table per scenario:

| Opcode | Avg (ns) | Total (ns) | Count | % of Total |
|--------|----------|------------|-------|------------|

- Sortable by any column
- Scenario selector dropdown

**Data source**: `BenchResult.opcode_timings` array from `*-bench.json`.

### 4.6 Regression Alerts (`/regressions`)

**Purpose**: Track which commits caused performance changes.

**Content**:
- Timeline view: commits with regression/stable/improvement badges
- For each flagged commit:
  - Which scenario/opcode regressed
  - Baseline vs current values
  - Percentage change
  - Link to the GitHub commit / PR
- Thresholds displayed: Warning at 20%, Regression at 50% (from `Thresholds::default()`)
- JIT speedup regression threshold: 20% drop

**Data source**: `*-regression.json` and `*-jit-regression.json` files. Status values from `RegressionStatus` enum: `Stable`, `Warning`, `Regression`.

---

## 5. API Endpoints

The v1.0 dashboard uses **no backend API**. All data is served as static JSON files from the data repository. The frontend fetches them client-side.

### Static File Endpoints

| Path | Description | Type |
|------|-------------|------|
| `/data/index.json` | Manifest of all benchmark runs | `DashboardIndex` |
| `/data/{date}/{commit}-bench.json` | Interpreter benchmark suite | `BenchSuite` |
| `/data/{date}/{commit}-jit-bench.json` | JIT benchmark suite | `JitBenchSuite` |
| `/data/{date}/{commit}-cross-client.json` | Cross-client comparison | `CrossClientSuite` |
| `/data/{date}/{commit}-regression.json` | Opcode regression report | `RegressionReport` |
| `/data/{date}/{commit}-jit-regression.json` | JIT speedup regression report | `JitRegressionReport` |

### Future API (v2.0, if needed)

If the data volume or query complexity outgrows static files (unlikely for months), a lightweight API could be added:

```
GET /api/v1/runs                        -> paginated list of runs
GET /api/v1/runs/:commit/bench          -> BenchSuite
GET /api/v1/runs/:commit/jit            -> JitBenchSuite
GET /api/v1/runs/:commit/cross-client   -> CrossClientSuite
GET /api/v1/scenarios/:name/history      -> time-series for one scenario
```

This would be a simple Rust binary reading from the same JSON files, or a Cloudflare Worker reading from R2 storage.

---

## 6. Deployment

### Domain

`clients.tokamak.network` -- CNAME to GitHub Pages or Cloudflare Pages.

### Hosting Options (ordered by preference)

| Option | Pros | Cons |
|--------|------|------|
| **Cloudflare Pages** (recommended) | Free, fast global CDN, deploy-on-push, custom domain easy | Requires Cloudflare account |
| GitHub Pages | Free, already using GitHub, data repo can be the site | 1GB size limit, slower CDN |
| Vercel | Free tier, good Next.js support | Vendor lock-in |

### Recommended Setup

1. **Data repository**: `tokamak-network/tokamak-dashboard-data` (public, GitHub Pages enabled)
   - Contains only JSON data files + `index.json` manifest
   - CI pushes new JSON files after each benchmark run
2. **Frontend repository**: `tokamak-network/tokamak-dashboard` (public)
   - Static site built with Next.js (static export) or Astro
   - Deployed to Cloudflare Pages on push to `main`
   - Fetches data from the data repository's GitHub Pages URL at runtime

### Alternative: Monorepo Approach

Keep everything in `ethrex` repo under `dashboard/`:
- `dashboard/data/` -- JSON files (gitignored locally, published via CI)
- `dashboard/site/` -- Frontend source
- CI publishes to a `gh-pages` branch

---

## 7. Tech Stack Recommendation

### Frontend

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| Framework | **Astro** (preferred) or Next.js static export | Astro ships zero JS by default, ideal for data display; Next.js if team prefers React |
| UI Components | React (via Astro islands) or Preact | Charting libraries need JS interactivity |
| Charting | **Recharts** or Chart.js | Recharts integrates natively with React; Chart.js is lighter |
| Styling | Tailwind CSS | Fast iteration, consistent design |
| TypeScript | Required | Type safety for JSON schemas |

### Data Pipeline

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| CI Runner | GitHub Actions (existing) | Already configured in `pr-tokamak-bench.yaml` |
| Data Push | `gh-pages` deploy action or API call | Minimal new infrastructure |
| Data Format | JSON (existing serde output) | Zero conversion needed -- types already have `Serialize`/`Deserialize` |

### TypeScript Type Definitions

Generate from the Rust types for type safety. The dashboard should include TS interfaces matching:

```typescript
// Mirrors crates/tokamak-bench/src/stats.rs :: BenchStats
interface BenchStats {
  mean_ns: number;
  stddev_ns: number;
  ci_lower_ns: number;
  ci_upper_ns: number;
  min_ns: number;
  max_ns: number;
  samples: number;
}

// Mirrors crates/tokamak-bench/src/types.rs :: OpcodeEntry
interface OpcodeEntry {
  opcode: string;
  avg_ns: number;
  total_ns: number;
  count: number;
}

// Mirrors crates/tokamak-bench/src/types.rs :: BenchResult
interface BenchResult {
  scenario: string;
  total_duration_ns: number;
  runs: number;
  opcode_timings: OpcodeEntry[];
  stats?: BenchStats;
}

// Mirrors crates/tokamak-bench/src/types.rs :: BenchSuite
interface BenchSuite {
  timestamp: string;
  commit: string;
  results: BenchResult[];
}

// Mirrors crates/tokamak-bench/src/types.rs :: JitBenchResult
interface JitBenchResult {
  scenario: string;
  interpreter_ns: number;
  jit_ns?: number;
  speedup?: number;
  runs: number;
  interp_stats?: BenchStats;
  jit_stats?: BenchStats;
}

// Mirrors crates/tokamak-bench/src/types.rs :: JitBenchSuite
interface JitBenchSuite {
  timestamp: string;
  commit: string;
  results: JitBenchResult[];
}

// Mirrors crates/tokamak-bench/src/cross_client/types.rs :: CrossClientResult
interface CrossClientResult {
  client_name: string;
  scenario: string;
  mean_ns: number;
  stats?: BenchStats;
}

// Mirrors crates/tokamak-bench/src/cross_client/types.rs :: CrossClientScenario
interface CrossClientScenario {
  scenario: string;
  results: CrossClientResult[];
  ethrex_mean_ns: number;
}

// Mirrors crates/tokamak-bench/src/cross_client/types.rs :: CrossClientSuite
interface CrossClientSuite {
  timestamp: string;
  commit: string;
  scenarios: CrossClientScenario[];
}

// Mirrors crates/tokamak-bench/src/types.rs :: RegressionStatus
type RegressionStatus = "Stable" | "Warning" | "Regression";

// Mirrors crates/tokamak-bench/src/types.rs :: Regression
interface Regression {
  scenario: string;
  opcode: string;
  baseline_avg_ns: number;
  current_avg_ns: number;
  change_percent: number;
}

// Mirrors crates/tokamak-bench/src/types.rs :: RegressionReport
interface RegressionReport {
  status: RegressionStatus;
  thresholds: { warning_percent: number; regression_percent: number };
  regressions: Regression[];
  improvements: Regression[];
}

// Mirrors crates/tokamak-bench/src/types.rs :: JitSpeedupDelta
interface JitSpeedupDelta {
  scenario: string;
  baseline_speedup: number;
  current_speedup: number;
  change_percent: number;
}

// Mirrors crates/tokamak-bench/src/types.rs :: JitRegressionReport
interface JitRegressionReport {
  status: RegressionStatus;
  threshold_percent: number;
  regressions: JitSpeedupDelta[];
  improvements: JitSpeedupDelta[];
}

// New: Dashboard-specific manifest
interface DashboardRun {
  date: string;
  commit: string;
  branch: string;
  files: {
    bench?: string;
    jit_bench?: string;
    cross_client?: string;
    regression?: string;
    jit_regression?: string;
  };
  status: RegressionStatus;
}

interface DashboardIndex {
  runs: DashboardRun[];
  latest_commit: string;
  total_runs: number;
}
```

---

## 8. Data Pipeline

### 8.1 Current CI Flow (already working)

The existing `pr-tokamak-bench.yaml` workflow already produces all the JSON artifacts needed. Current jobs:

| Job | Output | Type |
|-----|--------|------|
| `bench-pr` | `bench-pr.json` | `BenchSuite` |
| `bench-main` | `bench-main.json` | `BenchSuite` |
| `compare-results` | `comparison.json` | `RegressionReport` |
| `jit-bench-pr` | `jit-bench-pr.json` | `JitBenchSuite` |
| `jit-bench-main` | `jit-bench-main.json` | `JitBenchSuite` |
| `compare-jit-results` | `jit-report.md` | Markdown (needs JSON output too) |

### 8.2 Required CI Changes

Add a new job `publish-dashboard` that runs after all benchmark jobs complete:

```yaml
publish-dashboard:
  name: Publish to Dashboard
  runs-on: ubuntu-latest
  needs: [compare-results, compare-jit-results]
  if: >
    github.event.pull_request.merged == true ||
    github.ref == 'refs/heads/feat/tokamak-proven-execution'
  steps:
    - name: Download all artifacts
      uses: actions/download-artifact@v4
      with:
        path: ./artifacts

    - name: Checkout data repo
      uses: actions/checkout@v4
      with:
        repository: tokamak-network/tokamak-dashboard-data
        token: ${{ secrets.DASHBOARD_DEPLOY_TOKEN }}
        path: ./dashboard-data

    - name: Publish benchmark data
      run: |
        COMMIT="${{ github.event.pull_request.head.sha || github.sha }}"
        SHORT_COMMIT="${COMMIT:0:9}"
        DATE=$(date -u +%Y-%m-%d)
        DIR="dashboard-data/data/${DATE}"
        mkdir -p "${DIR}"

        # Copy available artifacts
        [ -f artifacts/bench-pr/bench-pr.json ] && \
          cp artifacts/bench-pr/bench-pr.json "${DIR}/${SHORT_COMMIT}-bench.json"
        [ -f artifacts/jit-bench-pr/jit-bench-pr.json ] && \
          cp artifacts/jit-bench-pr/jit-bench-pr.json "${DIR}/${SHORT_COMMIT}-jit-bench.json"

        # Rebuild index.json
        python3 scripts/rebuild-index.py dashboard-data/data/

    - name: Push to data repo
      working-directory: dashboard-data
      run: |
        git config user.name "github-actions[bot]"
        git config user.email "github-actions[bot]@users.noreply.github.com"
        git add -A
        git diff --cached --quiet || git commit -m "data: ${SHORT_COMMIT}"
        git push
```

### 8.3 Cross-Client Benchmark Pipeline

Cross-client benchmarks require running Geth/Reth nodes, so they run less frequently (weekly or on-demand):

```yaml
# New workflow: tokamak-cross-client-bench.yaml
name: Cross-Client Benchmark
on:
  schedule:
    - cron: "0 6 * * 1"  # Weekly, Monday 06:00 UTC
  workflow_dispatch: {}

jobs:
  cross-client:
    runs-on: ubuntu-latest
    services:
      geth:
        image: ethereum/client-go:latest
        ports: ["8546:8545"]
      reth:
        image: ghcr.io/paradigmxyz/reth:latest
        ports: ["8547:8545"]
    steps:
      - uses: actions/checkout@v4
      - uses: ./.github/actions/setup-rust
      - name: Build tokamak-bench
        run: cargo build --release -p tokamak-bench --features cross-client
      - name: Run cross-client benchmarks
        run: |
          target/release/tokamak-bench cross-client \
            --endpoints "geth=http://localhost:8546,reth=http://localhost:8547" \
            --runs 10 \
            --commit "${{ github.sha }}" \
            --output cross-client.json
      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: cross-client
          path: cross-client.json
      # ... publish-dashboard step similar to above
```

### 8.4 Data Flow Diagram

```
PR Merged to Branch
       |
       v
pr-tokamak-bench.yaml
  |                  |
  v                  v
bench-pr.json    jit-bench-pr.json
  |                  |
  v                  v
comparison.json  jit-regression.json
  |                  |
  +--------+---------+
           |
           v
  publish-dashboard job
           |
           v
  tokamak-dashboard-data repo
     (GitHub Pages)
           |
           v
  clients.tokamak.network
     (static frontend)
```

---

## 9. Implementation Phases

### Phase 1: MVP (8-12h)

**Goal**: Minimal working dashboard with latest results and historical trends.

**Tasks**:
1. Create `tokamak-dashboard-data` repository with GitHub Pages enabled
2. Write `rebuild-index.py` script that scans `data/` directories and generates `index.json`
3. Add `publish-dashboard` job to `pr-tokamak-bench.yaml` (only on merge to main branch)
4. Scaffold frontend project (Astro + Tailwind + TypeScript)
5. Implement Landing Page:
   - Key metric cards (JIT speedup, regression status)
   - Latest benchmark results table
6. Implement Historical Trends page:
   - Single line chart per scenario (mean execution time over commits)
   - Scenario selector dropdown
7. Deploy to `clients.tokamak.network`

**Deliverables**:
- Live site showing latest benchmark data
- Auto-publish on merge

### Phase 2: JIT + Opcode Detail (6-10h)

**Goal**: Full JIT comparison view and per-opcode breakdown.

**Tasks**:
1. JIT vs Interpreter page:
   - Side-by-side bar chart
   - Historical speedup trend line
   - Full statistics table
2. Per-Opcode Breakdown page:
   - Stacked bar chart (top 10 opcodes by time)
   - Sortable data table
3. Add error bands (95% CI) to trend charts
4. Add `jit-compare` JSON output to CI (currently only produces markdown)

### Phase 3: Cross-Client + Regressions (6-8h)

**Goal**: Cross-client comparison and regression alerting.

**Tasks**:
1. Cross-Client Comparison page:
   - Grouped bar chart
   - Ratio table with ethrex as 1.00x baseline
   - Methodology caveat (in-process vs RPC)
2. Regression Alerts page:
   - Timeline view with status badges per commit
   - Drill-down into flagged commits
3. Set up weekly cross-client benchmark workflow
4. Add RSS/Atom feed for regression alerts (optional)

### Phase 4: Polish (2-4h)

**Goal**: Production-ready quality.

**Tasks**:
1. Responsive design (mobile-friendly)
2. Dark mode support
3. SEO metadata and Open Graph tags
4. Favicon and branding (Tokamak logo)
5. "About" page explaining methodology
6. Link to source code (ethrex repo, tokamak-bench crate)

---

## 10. Open Questions

| # | Question | Options | Impact |
|---|----------|---------|--------|
| 1 | **Separate data repo or monorepo?** | (a) `tokamak-dashboard-data` separate repo (b) `ethrex` repo `gh-pages` branch (c) S3/R2 bucket | Affects CI setup complexity and data management. Separate repo is cleanest. |
| 2 | **Trigger: every PR merge or only to main?** | (a) Every merge to `feat/tokamak-proven-execution` (b) Only merges to `main` (c) Nightly schedule | Frequent updates are better for trends but cost CI minutes. Recommend (a) for now, switch to (b) when branch merges to main. |
| 3 | **Cross-client fairness caveat** | How prominently should we note that Geth/Reth measurements include RPC overhead while ethrex runs in-process? | Critical for credibility. Must be clearly visible on the comparison page. |
| 4 | **Data retention policy** | Keep all historical data forever? Trim to last 90 days? | JSON files are small (~10KB each). Recommend keeping all data. Set a re-evaluation threshold at 10,000 runs. |
| 5 | **Authentication for cross-client runs** | Cross-client benchmarks need running Geth/Reth nodes. Use GitHub Actions services (containers) or external hosted nodes? | Containers are reproducible but may have different performance characteristics than production nodes. |
| 6 | **Hive/sync data integration** | Should the dashboard also display Hive pass rates and sync times? These are not currently exported as JSON. | Nice-to-have for Phase 4. Would require adding JSON output to `pr-tokamak.yaml` Hive jobs. |
| 7 | **Frontend framework final pick** | Astro (lighter, zero-JS default) vs Next.js (team familiarity, richer ecosystem) | Both work. Astro is recommended for this use case since the site is mostly static data display with a few interactive charts. |
| 8 | **DASHBOARD_DEPLOY_TOKEN secret** | Use a fine-grained PAT, a GitHub App token, or `GITHUB_TOKEN` with cross-repo permissions? | GitHub App token is most secure. PAT is simplest for initial setup. |

---

## Appendix A: Existing Code References

| File | Relevance |
|------|-----------|
| `crates/tokamak-bench/src/types.rs` | All benchmark data types (`BenchSuite`, `JitBenchSuite`, `RegressionReport`, etc.) |
| `crates/tokamak-bench/src/stats.rs` | `BenchStats` struct, `compute_stats()`, `split_warmup()` |
| `crates/tokamak-bench/src/report.rs` | JSON/markdown serialization (`to_json`, `from_json`, `to_markdown`, `jit_to_markdown`) |
| `crates/tokamak-bench/src/regression.rs` | `compare()` and `compare_jit()` regression detection |
| `crates/tokamak-bench/src/cross_client/types.rs` | `CrossClientSuite`, `CrossClientResult`, `CrossClientScenario` |
| `crates/tokamak-bench/src/cross_client/report.rs` | Cross-client JSON/markdown report generation |
| `crates/tokamak-bench/src/cross_client/runner.rs` | `run_cross_client_suite()`, `eth_call` with state overrides |
| `crates/tokamak-bench/src/runner.rs` | `run_suite()`, `run_scenario()`, `default_scenarios()`, 12 benchmark scenarios |
| `.github/workflows/pr-tokamak-bench.yaml` | Existing CI pipeline: 6 jobs producing benchmark artifacts |

## Appendix B: Benchmark Scenarios

From `crates/tokamak-bench/src/runner.rs :: default_scenarios()`:

| Scenario | Iterations | JIT Status | Notes |
|----------|-----------|------------|-------|
| Fibonacci | 57 | JIT available | Primary JIT benchmark (2.53x speedup) |
| FibonacciRecursive | 15 | Skipped | Recursive CALL suspend/resume too slow (D-1) |
| Factorial | 57 | JIT available | 1.67x speedup |
| FactorialRecursive | 57 | Skipped | Same as FibonacciRecursive |
| Push | 0 | Interpreter-only | Bytecode > 24KB (D-2 fallback) |
| MstoreBench | 0 | Interpreter-only | Bytecode > 24KB |
| SstoreBench_no_opt | 0 | Interpreter-only | Bytecode > 24KB |
| ManyHashes | 57 | JIT available | 1.46x speedup |
| BubbleSort | 100 | JIT available | 2.24x speedup |
| ERC20Approval | 500 | Skipped | Recursive CALL |
| ERC20Transfer | 500 | Skipped | Recursive CALL |
| ERC20Mint | 500 | Skipped | Recursive CALL |

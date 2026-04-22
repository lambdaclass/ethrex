# Observability & Reporting

This document covers dashboard generation, live monitoring, and notification setup for benchmarking sessions.

---

## Report Structure

Generate a static HTML report for visibility into benchmark progress:

```
docs/perf/report/
├── index.html          # Main dashboard
├── style.css           # Styling (optional, can be inline)
├── data.json           # All results data
├── charts/             # Generated visualizations (optional)
└── experiments/        # Per-experiment detail pages
    ├── 001.html
    └── 002.html
```

---

## Dashboard Contents

The dashboard (`index.html`) should include:

### 1. Overview Section

```html
<div class="overview">
  <h2>Overview</h2>
  <p>Status: <span class="status-running">Running</span></p>
  <p>Progress: 3/12 experiments complete</p>
  <p>Best improvement: -6.5% (experiment 001)</p>
  <p>Time elapsed: 2h 15m</p>
</div>
```

### 2. Baseline Section

```html
<div class="baseline">
  <h2>Baseline</h2>
  <table>
    <tr><th>Metric</th><th>Value</th><th>StdDev</th></tr>
    <tr><td>Block time</td><td>12.68ms</td><td>±0.5ms</td></tr>
    <tr><td>Throughput</td><td>669 Mgas/s</td><td>±15</td></tr>
  </table>
  <details>
    <summary>Environment</summary>
    <pre>Machine: ethrex-office-2
CPU: AMD Ryzen 9 5950X
Memory: 64GB
Commit: abc123</pre>
  </details>
</div>
```

### 3. Experiments Table

```html
<div class="experiments">
  <h2>Experiments</h2>
  <table>
    <tr>
      <th>#</th>
      <th>Name</th>
      <th>Time</th>
      <th>Δ%</th>
      <th>Status</th>
      <th>Notes</th>
    </tr>
    <tr class="keep">
      <td>001</td>
      <td>Skip memory zero-init</td>
      <td>11.85ms</td>
      <td>-6.5%</td>
      <td>KEEP</td>
      <td>Merged to main</td>
    </tr>
    <tr class="discard">
      <td>002</td>
      <td>Zero-copy deserialization</td>
      <td>12.58ms</td>
      <td>-0.8%</td>
      <td>DISCARD</td>
      <td>Below threshold</td>
    </tr>
    <tr class="running">
      <td>003</td>
      <td>FxHashSet access lists</td>
      <td>---</td>
      <td>---</td>
      <td>RUNNING</td>
      <td>Run 5/10</td>
    </tr>
  </table>
</div>
```

### 4. Trend Chart (Optional)

If generating charts, show performance trend:
- X-axis: experiment number
- Y-axis: performance metric (time or throughput)
- Horizontal line: baseline

### 5. Current Run (if active)

```html
<div class="current-run">
  <h2>Current Run</h2>
  <p>Experiment: 003 - FxHashSet access lists</p>
  <p>Progress: Run 5/10</p>
  <pre class="live-log">
[2026-01-16 14:32:01] Starting run 5...
[2026-01-16 14:32:15] Run 5 complete: 11.42ms
  </pre>
</div>
```

### 6. Learnings Section

```html
<div class="learnings">
  <h2>Key Learnings</h2>
  <ul>
    <li>Memory zero-init accounts for ~6% of block time</li>
    <li>Access list operations are in hot path for every SLOAD/SSTORE</li>
  </ul>
</div>
```

---

## Auto-Refresh

For live monitoring during active benchmarking:

### HTML Meta Tag (Simple)

```html
<!-- Add to index.html <head> -->
<meta http-equiv="refresh" content="30">
```

### JavaScript (Smoother)

```javascript
// Add to index.html
<script>
  // Refresh every 30 seconds
  setTimeout(() => location.reload(), 30000);

  // Or fetch data without full reload
  async function updateData() {
    const response = await fetch('data.json');
    const data = await response.json();
    // Update DOM elements
  }
  setInterval(updateData, 30000);
</script>
```

---

## Serving the Report

### Local Development

```bash
# Simple HTTP server on the benchmark machine
cd docs/perf/report && python3 -m http.server 8080

# Or with specific binding
python3 -m http.server 9999 --bind 0.0.0.0
```

### Access from Other Machines

```bash
# Find machine IP
hostname -I

# Report will be available at:
# http://<machine-ip>:8080/
```

---

## Report Generation

Use the [generate_report.py](scripts/generate_report.py) script to create the HTML dashboard:

```bash
# Generate report from experiment data
python3 docs/perf/scripts/generate_report.py

# Or with custom paths
python3 docs/perf/scripts/generate_report.py \
  --experiments docs/perf/experiments \
  --output docs/perf/report
```

---

## Notifications

### Slack Integration (Optional)

Store webhook URL securely:

```bash
# On the benchmark server, create:
mkdir -p ~/.config/benchmarks
echo "SLACK_WEBHOOK=https://hooks.slack.com/..." > ~/.config/benchmarks/secrets.env
chmod 600 ~/.config/benchmarks/secrets.env
```

Notification script:

```bash
#!/bin/bash
# ~/.local/bin/notify-benchmark
source ~/.config/benchmarks/secrets.env

MESSAGE="$1"
curl -s -X POST "$SLACK_WEBHOOK" \
  -H 'Content-Type: application/json' \
  -d "{\"text\": \"$MESSAGE\"}"
```

### When to Notify

| Event | Priority | Example Message |
|-------|----------|-----------------|
| Baseline complete | Medium | "Baseline established: 12.68ms/block" |
| Experiment complete | Low | "Exp 001: -6.5% KEEP" |
| Significant finding (>10%) | High | "Major improvement: 001 shows -15%!" |
| Error or crash | High | "ERROR: Exp 003 crashed - stack overflow" |
| Session complete | Medium | "All experiments done. Best: -6.5%" |

### Message Format

```
Experiment 001 Complete
---
Name: Skip memory zero-init
Result: 11.85ms (baseline: 12.68ms)
Improvement: -6.5%
Status: KEEP
---
Progress: 3/12 experiments
Dashboard: http://ethrex-office-2:8080/
```

---

## Data Format (data.json)

Structure for dashboard data:

```json
{
  "session": {
    "date": "2026-01-16",
    "status": "running",
    "commit": "abc123",
    "machine": "ethrex-office-2"
  },
  "baseline": {
    "time_ms": 12.68,
    "time_stddev": 0.5,
    "throughput_mgas_s": 669,
    "runs": 10
  },
  "experiments": [
    {
      "id": "001",
      "name": "Skip memory zero-init",
      "status": "keep",
      "time_ms": 11.85,
      "improvement_percent": -6.5,
      "notes": "Merged to main"
    },
    {
      "id": "002",
      "name": "Zero-copy deserialization",
      "status": "discard",
      "time_ms": 12.58,
      "improvement_percent": -0.8,
      "notes": "Below threshold"
    },
    {
      "id": "003",
      "name": "FxHashSet access lists",
      "status": "running",
      "current_run": 5,
      "total_runs": 10
    }
  ],
  "learnings": [
    "Memory zero-init accounts for ~6% of block time",
    "Access list operations are in hot path"
  ]
}
```

---

## Styling (Optional)

Minimal CSS for the dashboard:

```css
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  max-width: 1200px;
  margin: 2rem auto;
  padding: 0 1rem;
}

.card {
  border: 1px solid #ddd;
  border-radius: 8px;
  padding: 1rem;
  margin: 1rem 0;
}

table {
  width: 100%;
  border-collapse: collapse;
}

th, td {
  border: 1px solid #ddd;
  padding: 8px;
  text-align: left;
}

th {
  background: #f5f5f5;
}

.keep { background: #d4edda; }
.discard { background: #f8d7da; }
.running { background: #fff3cd; }
.failed { background: #f5c6cb; }

.status-running { color: #856404; font-weight: bold; }
.status-complete { color: #155724; font-weight: bold; }
```

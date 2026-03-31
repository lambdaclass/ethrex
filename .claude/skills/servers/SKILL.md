---
name: servers
description: "Manage ethrex testing servers: status reports, rerun (keep DB), restart (clean DB). Use `/servers` for status, `/servers rerun srv5 main` to switch branch, `/servers restart srv3 main` to clean and switch."
---

# Server Management Skill

Manage ethrex testing servers: status reports with fair performance comparisons, and deploy branches to servers.

## Subcommands

### `/servers` (no args) — Status report

Execute the status script:

```bash
bash ~/.claude/skills/servers/server-status.sh
```

**What to do with the output:**

1. **Echo the markdown output verbatim** — the user wants to see the rendered tables every time
2. **Highlight anomalies** after the tables:
   - Unreachable servers
   - Very low common block counts (< 10 blocks means comparison is unreliable)
   - Large regressions (> 10% slower throughput or > 10% higher block time)
   - Servers stuck in `syncing` or `stopped` state
   - Servers where branch/commit doesn't match what's expected
3. **Note cold-cache servers** — recently restarted servers (low block count relative to others) may show poor numbers until caches stabilize (~30+ min)

### `/servers rerun <server> <branch>` — Switch branch, keep DB

Stops ethrex, switches to the given branch, builds, and restarts **without cleaning the database**.

```bash
bash ~/.claude/skills/servers/server-manage.sh rerun <server> <branch>
```

Example: `/servers rerun srv5 replace-ethrex-rlp-with-librlp`

### `/servers restart <server> <branch>` — Switch branch, clean DB

Stops ethrex, **deletes the database**, switches to the given branch, builds, and restarts from scratch.

```bash
bash ~/.claude/skills/servers/server-manage.sh restart <server> <branch>
```

Example: `/servers restart srv3 main`

**Warning:** This wipes `~/.local/share/ethrex` on the server. The node will sync from genesis which takes a very long time.

## Server Names

| Short name | SSH host |
|------------|----------|
| srv1..srv10 | `admin@ethrex-mainnet-{1-10}` |
| office1..office5 | `admin@ethrex-office-{1-5}` |

## Server Groups

| Group | Servers | Baseline | Notes |
|-------|---------|----------|-------|
| Instance A | srv1–srv5 | srv1 | `admin@ethrex-mainnet-{1-5}` |
| Instance B | srv6–srv10 | srv6 | `admin@ethrex-mainnet-{6-10}` |
| Office | office1–office5 | none | `admin@ethrex-office-{1-5}`, no baseline |

## Key Concepts

- **Block intersection**: Stats are computed only on blocks processed by ALL running servers in a group. If srv4 just restarted, all servers' stats are restricted to srv4's blocks.
- **Steady-state detection**: Skips the initial catch-up burst (blocks processed < 5s apart) so comparisons reflect real-time execution, not replay speed.
- **Percentiles**: p50 (median), p95, p99 for both throughput (Ggas/s) and block time (ms).
- **Deltas**: Test servers show `value (+X.X%)` relative to their group's baseline. For throughput, positive is better. For block time, negative is better.

## Troubleshooting

- If all servers are unreachable: check SSH keys and network (BatchMode=yes means no password prompts)
- If common blocks = 0 but servers are running: they may have non-overlapping block ranges (different restart times or sync states)
- Script timeout: SSH has 5s connect timeout per server; total runtime ~10-15s typical

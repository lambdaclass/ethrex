# On-demand GPU CI runner

The `gpu` self-hosted runner (`ethrex-gpu-ci`) is a Paperspace machine billed by
the hour. It only needs to be on while `main_prover.yaml` runs, so:

- **Start:** the `start-gpu-runner` job in `.github/workflows/main_prover.yaml`
  starts the machine through the Paperspace API, using
  `.github/actions/start-paperspace-machine`. The `test` job needs it, then
  waits in the queue until the runner comes online.
- **Stop:** `idle-stop.sh` runs every minute on the machine, triggered by a
  systemd timer. It stops the machine once no job has run, and no GPU workflow
  run has been queued or in progress, for `IDLE_MINUTES` (default 15).

15 minutes was picked from October 2026 job data. Most gaps between jobs are
under 5 minutes, because runs are queued back to back. Going from 15 to
60 minutes would have avoided only 5 more cold starts in the month, at about
$45 of extra compute.

## Files

| File | What it does |
|---|---|
| `paperspace-machine.sh` | `state` / `start` / `stop` through the Paperspace API. `start` is idempotent and waits until the machine is ready. |
| `idle-stop.sh` | Idle watchdog. It stops the machine with `paperspace-machine.sh stop`. |
| `gpu-runner-idle.service`, `gpu-runner-idle.timer` | systemd units that run the watchdog every minute. |

## Setup

### GitHub
- Secret `PAPERSPACE_API_KEY`: a Paperspace API key, created under Team settings -> API keys.
- Variable `GPU_RUNNER_MACHINE_ID`: the Paperspace machine ID, e.g. `pszj5x2xbgeb`.

### On the machine
1. Make sure the Actions runner is a service, so it comes back online after every boot:
   ```bash
   cd /home/admin/actions-runner && sudo ./svc.sh install && sudo ./svc.sh start
   ```
2. Install the watchdog:
   ```bash
   sudo install -d /opt/gpu-runner
   sudo install -m 755 tooling/gpu-runner/paperspace-machine.sh tooling/gpu-runner/idle-stop.sh /opt/gpu-runner/
   sudo install -m 644 tooling/gpu-runner/gpu-runner-idle.service tooling/gpu-runner/gpu-runner-idle.timer /etc/systemd/system/
   ```
3. Create `/etc/gpu-runner-idle.env`, readable only by root (`chmod 600`):
   ```
   PAPERSPACE_API_KEY=...
   PAPERSPACE_MACHINE_ID=pszj5x2xbgeb
   # Fine-grained token, Actions: read on lambdaclass/ethrex
   GITHUB_TOKEN=...
   IDLE_MINUTES=15
   # Set to 0 once the logs look right
   DRY_RUN=1
   ```
   Keep comments on their own lines: systemd's `EnvironmentFile` treats a
   trailing `# ...` as part of the value.
4. Enable the timer and watch it:
   ```bash
   sudo systemctl daemon-reload && sudo systemctl enable --now gpu-runner-idle.timer
   journalctl -u gpu-runner-idle -f
   ```

## Notes
- When the machine is off, only compute stops billing. Disk, public IP and the
  custom template are still charged.
- Each cold start delays a run by the boot time plus the time the runner takes to come online.
- If the GitHub API can't be reached, the watchdog keeps the machine on. As a
  backstop, set Paperspace auto-shutdown in the console well above the longest
  job (e.g. 6h).
- A run stuck in the queue keeps the machine on until GitHub drops it. GitHub
  drops queued jobs after 24h.

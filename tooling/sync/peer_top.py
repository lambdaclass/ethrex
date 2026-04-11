#!/usr/bin/env python3
"""Live peer table viewer — like top for ethrex peers."""
import os, signal, sys, time
import requests as req

ENDPOINT = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:18547"
INTERVAL = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0

# ANSI colors
RED = "\033[31m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
DIM = "\033[2m"
BOLD = "\033[1m"
RESET = "\033[0m"

# Track previous scores for delta coloring
prev_scores: dict[str, int] = {}


def fetch(method):
    try:
        r = req.post(
            ENDPOINT,
            json={"jsonrpc": "2.0", "method": method, "params": [], "id": 1},
            timeout=3,
        )
        return r.json().get("result")
    except Exception:
        return None


start_time = time.time()


def color_score(peer_id: str, score: int) -> str:
    """Color the score based on value and delta from previous tick."""
    prev = prev_scores.get(peer_id)
    if score <= -30:
        color = RED
    elif score <= 0:
        color = YELLOW
    else:
        color = GREEN

    if prev is not None and prev != score:
        if score > prev:
            # Score went up — bright green arrow
            return f"{GREEN}{BOLD}{score:>4} \u2191{RESET}"
        else:
            # Score went down — bright red arrow
            return f"{RED}{BOLD}{score:>4} \u2193{RESET}"
    return f"{color}{score:>4}  {RESET}"


def render():
    global prev_scores
    lines = []
    elapsed = int(time.time() - start_time)
    h, m, s = elapsed // 3600, (elapsed % 3600) // 60, elapsed % 60
    now_str = time.strftime("%H:%M:%S")
    lines.append(
        f"{BOLD}peer_top{RESET} {DIM}— {now_str} — up {h:02d}:{m:02d}:{s:02d} — {ENDPOINT}{RESET}"
    )
    lines.append("")
    sync = fetch("admin_syncStatus")
    data = fetch("admin_peerScores")

    if sync:
        phase = sync.get("current_phase") or "idle"
        pivot = sync.get("pivot_block_number") or "?"
        age = sync.get("pivot_age_seconds")
        threshold = sync.get("staleness_threshold_seconds", 0)
        progress = sync.get("phase_progress", {})
        age_str = f"{age}s" if age else "?"

        # Color staleness margin
        if age and threshold:
            margin_secs = threshold - age
            if margin_secs < 0:
                margin_color = RED
            elif margin_secs < 300:
                margin_color = YELLOW
            else:
                margin_color = GREEN
            margin = f"{margin_color}({margin_secs}s to stale){RESET}"
        else:
            margin = ""

        lines.append(
            f"{BOLD}Phase:{RESET} {CYAN}{phase}{RESET}  "
            f"{BOLD}Pivot:{RESET} {pivot}  "
            f"{BOLD}Age:{RESET} {age_str}  {margin}"
        )
        if progress:
            parts = [f"{k}={v:,}" for k, v in progress.items()]
            lines.append(f"{DIM}Progress: {', '.join(parts)}{RESET}")

        # Pivot update history
        pivot_changes = sync.get("recent_pivot_changes", [])
        if pivot_changes:
            lines.append("")
            lines.append(f"{BOLD}Pivot History:{RESET} (last {len(pivot_changes)})")
            for pc in pivot_changes[-5:]:  # show last 5
                ts = pc.get("timestamp", 0)
                ts_str = time.strftime("%H:%M:%S", time.localtime(ts)) if ts else "?"
                old_n = pc.get("old_pivot_number", "?")
                new_n = pc.get("new_pivot_number", "?")
                outcome = pc.get("outcome", "?")
                reason = pc.get("failure_reason", "")
                if outcome == "success":
                    icon = f"{GREEN}\u2713{RESET}"
                else:
                    icon = f"{RED}\u2717{RESET}"
                    if reason:
                        reason = f" {RED}{reason}{RESET}"
                lines.append(
                    f"  {icon} {DIM}{ts_str}{RESET} {old_n} \u2192 {new_n} [{outcome}]{reason}"
                )

        # Recent errors
        errors = sync.get("recent_errors", [])
        if errors:
            lines.append("")
            lines.append(f"{BOLD}Recent Errors:{RESET} (last {len(errors)})")
            for err in errors[-3:]:  # show last 3
                ts = err.get("timestamp", 0)
                ts_str = time.strftime("%H:%M:%S", time.localtime(ts)) if ts else "?"
                msg = err.get("error_message", "?")[:60]
                recov = f"{GREEN}recoverable{RESET}" if err.get("recoverable") else f"{RED}irrecoverable{RESET}"
                lines.append(f"  {DIM}{ts_str}{RESET} {msg} [{recov}]")

        lines.append("")

    if not data:
        lines.append(f"{RED}Node not reachable{RESET}")
        return lines

    s = data["summary"]
    peers = data["peers"]

    # Color eligible count
    elig_count = s["eligible_peers"]
    if elig_count < 5:
        elig_color = RED
    elif elig_count < 20:
        elig_color = YELLOW
    else:
        elig_color = GREEN

    lines.append(
        f"{BOLD}Peers:{RESET} {s['total_peers']}  "
        f"{BOLD}Eligible:{RESET} {elig_color}{elig_count}{RESET}  "
        f"{BOLD}Avg Score:{RESET} {s['average_score']}  "
        f"{BOLD}Inflight:{RESET} {s['total_inflight_requests']}"
    )
    lines.append("")
    lines.append(
        f"{DIM}{'Peer ID':>14} {'Score':>6} {'Reqs':>5} {'Elig':>5}"
        f" {'Capabilities':>22} {'Dir':>4} {'Client':>35}{RESET}"
    )
    lines.append(f"{DIM}{'-' * 97}{RESET}")

    new_scores = {}
    for p in sorted(peers, key=lambda x: x["score"], reverse=True):
        pid_full = p["peer_id"]
        pid = pid_full[:6] + ".." + pid_full[-4:]
        score = p["score"]
        new_scores[pid_full] = score

        score_str = color_score(pid_full, score)

        # Group capabilities by protocol
        by_proto = {}
        for c in p["capabilities"]:
            parts = c.split("/")
            proto = parts[0]
            ver = parts[1] if len(parts) > 1 else "?"
            by_proto.setdefault(proto, []).append(ver)
        caps = " ".join(f"{k}/{','.join(vs)}" for k, vs in by_proto.items())
        client = p["client_version"][:35]
        d = p["connection_direction"][:3]

        if p["eligible"]:
            elig = f"{GREEN}\u2713{RESET}"
        else:
            elig = f"{RED}\u2717{RESET}"

        reqs = p["inflight_requests"]
        reqs_str = f"{YELLOW}{reqs:>5}{RESET}" if reqs > 0 else f"{reqs:>5}"

        lines.append(
            f"{pid:>14} {score_str} {reqs_str}"
            f" {elig:>14} {caps:>22} {d:>4} {DIM}{client:>35}{RESET}"
        )

    prev_scores = new_scores
    return lines


def cleanup(*_):
    sys.stdout.write("\033[?1049l\033[?25h")
    sys.stdout.flush()
    sys.exit(0)


signal.signal(signal.SIGINT, cleanup)
signal.signal(signal.SIGTERM, cleanup)

sys.stdout.write("\033[?1049h\033[?25l\033[2J")
sys.stdout.flush()

try:
    prev_line_count = 0
    while True:
        lines = render()
        try:
            term_rows = os.get_terminal_size().lines
        except OSError:
            term_rows = 40
        if len(lines) > term_rows - 2:
            hidden = len(lines) - term_rows + 3
            lines = lines[: term_rows - 3]
            lines.append(f"  {DIM}... {hidden} more peers (resize terminal to see all){RESET}")
        buf = "\033[H"
        for line in lines:
            buf += f"{line}\033[K\n"
        for _ in range(max(0, prev_line_count - len(lines))):
            buf += "\033[K\n"
        sys.stdout.write(buf)
        sys.stdout.flush()
        prev_line_count = len(lines)
        time.sleep(INTERVAL)
except Exception:
    cleanup()

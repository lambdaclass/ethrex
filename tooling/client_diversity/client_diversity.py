#!/usr/bin/env python3
"""Estimate execution client share of proposed blocks from block graffiti.

Consensus clients append a short client-version tag to the graffiti of the
blocks they propose, built from the `engine_getClientVersionV1` response of
their execution client (e.g. `GE1df3LH57c2` = Geth commit 1df3 + Lighthouse
commit 57c2). This script fetches every block in a time range from a beacon
node, tallies the execution client code found in each graffiti, plots how the
share of one client (ethrex by default) evolved over the range, and estimates
the stake running on that client as

    total active stake * (blocks tagged with that client / total blocks)

Usage:
    client_diversity.py [--range 30d] [--beacon-url http://localhost:5052]
                        [--client ethrex] [--bucket 1d] [--concurrency 16]

Pure stdlib; no third-party dependencies.
"""
from __future__ import annotations

import argparse
import datetime as dt
import http.client
import json
import math
import re
import sys
import threading
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from urllib.parse import urlsplit

# Client codes from the execution-apis spec (src/engine/identification.md).
EL_CODES = {
    "GE": "Geth",
    "NM": "Nethermind",
    "BU": "Besu",
    "RH": "Reth",
    "EG": "Erigon",
    "EX": "ethrex",
    "EJ": "EthereumJS",
    "NE": "Nimbus EL",
    "TE": "Trin Execution",
}
# Consensus client codes. Only used to validate that a token is a client tag.
CL_CODES = {
    "LH",  # Lighthouse
    "PM",  # Prysm
    "TK",  # Teku
    "NB",  # Nimbus
    "LS",  # Lodestar
    "GR",  # Grandine
    "CN",  # Caplin
    "LA",  # ethlambda
    "PR",  # not in the spec; seen on mainnet with the commit prefix of a Prysm v7.1.4 build
}

_HEX = r"(?:[0-9a-fA-F]{4}|[0-9a-fA-F]{2})?"
_TAG = rf"([A-Z]{{2}}){_HEX}([A-Z]{{2}}){_HEX}"
_TAG_LOOKAHEAD = rf"[A-Z]{{2}}{_HEX}[A-Z]{{2}}{_HEX}$"
# EL code [commit] CL code [commit], 4/8/12 chars. Either a standalone word, or
# glued to the end of the user graffiti with no separator (seen on mainnet).
TAG_RE = re.compile(rf"(?:(?<![A-Za-z0-9])|(?={_TAG_LOOKAHEAD})){_TAG}(?![A-Za-z0-9])")
# When only 2 bytes are left, clients append just the EL code.
EL_ONLY_RE = re.compile(r"(?:^|\s)([A-Z]{2})$")
EFFECTIVE_BALANCE_RE = re.compile(rb'"effective_balance"\s*:\s*"(\d+)"')

# A run of empty slots this long at the start of the range means the node does
# not have those blocks (e.g. checkpoint sync without full backfill).
MAX_EMPTY_RUN = 64
CHUNK = 64
CHART_HEIGHT = 10
CHART_WIDTH = 60
CHART_MAX_BUCKETS = 40
UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}
BUCKET_SIZES = [3600 * h for h in (1, 2, 3, 6, 12)] + [86400 * d for d in (1, 2, 3, 7, 14, 30)]


def el_code_from_graffiti(graffiti_hex: str) -> str | None:
    """Return the execution client code in a 0x-prefixed graffiti, if any."""
    text = bytes.fromhex(graffiti_hex[2:]).rstrip(b"\x00").decode("utf-8", "replace")
    code = None
    for m in TAG_RE.finditer(text):
        el, cl = m.groups()
        if el in EL_CODES and cl in CL_CODES:
            code = el
    if code is None:
        m = EL_ONLY_RE.search(text)
        if m and m.group(1) in EL_CODES:
            code = m.group(1)
    return code


class BeaconClient:
    """Minimal beacon API client with one keep-alive connection per thread."""

    def __init__(self, url: str):
        u = urlsplit(url)
        if u.scheme not in ("http", "https"):
            raise ValueError(f"unsupported beacon URL: {url}")
        self.scheme, self.host, self.port = u.scheme, u.hostname, u.port
        self.prefix = u.path.rstrip("/")
        self.local = threading.local()

    def _conn(self) -> http.client.HTTPConnection:
        c = getattr(self.local, "conn", None)
        if c is None:
            cls = http.client.HTTPSConnection if self.scheme == "https" else http.client.HTTPConnection
            c = self.local.conn = cls(self.host, self.port, timeout=120)
        return c

    def _drop_conn(self):
        c = getattr(self.local, "conn", None)
        if c is not None:
            c.close()
        self.local.conn = None

    def request(self, path: str, retries: int = 5) -> http.client.HTTPResponse:
        """GET path and return the response with its body unread."""
        for attempt in range(retries + 1):
            try:
                c = self._conn()
                c.request("GET", self.prefix + path, headers={"Accept": "application/json"})
                return c.getresponse()
            except (OSError, http.client.HTTPException):
                self._drop_conn()
                if attempt == retries:
                    raise
                time.sleep(0.5 * (attempt + 1))

    def get_json(self, path: str, retries: int = 5):
        """GET path as JSON. Returns None on 404, raises on other errors."""
        for attempt in range(retries + 1):
            try:
                r = self.request(path)
                body = r.read()
                if r.status == 404:
                    return None
                if r.status == 200:
                    return json.loads(body)
                err = RuntimeError(f"GET {path}: HTTP {r.status}: {body[:200]!r}")
            except (OSError, http.client.HTTPException, ValueError) as e:
                self._drop_conn()
                err = e
            if attempt == retries:
                raise err
            time.sleep(0.5 * (attempt + 1))


def parse_duration(s: str) -> int:
    """Parse a duration like 30d, 12h, 2w, 90m into seconds."""
    m = re.fullmatch(r"(\d+)\s*([smhdw])", s.strip())
    if not m or int(m.group(1)) == 0:
        raise argparse.ArgumentTypeError(f"invalid duration {s!r}, expected e.g. 30d, 12h, 2w")
    return int(m.group(1)) * UNITS[m.group(2)]


def fmt_duration(seconds: int) -> str:
    for unit in ("w", "d", "h", "m"):
        if seconds % UNITS[unit] == 0:
            return f"{seconds // UNITS[unit]}{unit}"
    return f"{seconds}s"


def fmt_time(ts: int, fmt: str = "%Y-%m-%d %H:%M UTC") -> str:
    return dt.datetime.fromtimestamp(ts, dt.timezone.utc).strftime(fmt)


def total_active_stake(beacon: BeaconClient) -> tuple[int, int]:
    """Sum the effective balance (gwei) of active validators at the finalized state.

    The response is several hundred MB on mainnet, so it is scanned in chunks
    instead of being parsed as one JSON document.
    """
    r = beacon.request("/eth/v1/beacon/states/finalized/validators?status=active")
    if r.status != 200:
        raise RuntimeError(f"validators request failed: HTTP {r.status}: {r.read()[:200]!r}")
    total = count = 0
    buf = b""
    while True:
        chunk = r.read(1 << 20)
        if not chunk:
            break
        buf += chunk
        last = 0
        for m in EFFECTIVE_BALANCE_RE.finditer(buf):
            total += int(m.group(1))
            count += 1
            last = m.end()
        buf = buf[max(last, len(buf) - 64):]
    return total, count


def nice_ceiling(x: float) -> float:
    """Smallest 'round' number (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 x 10^k) >= x."""
    if x <= 0:
        return 1.0
    exp = math.floor(math.log10(x))
    for m in (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10):
        if m * 10 ** exp >= x * (1 - 1e-9):
            return m * 10 ** exp
    return 10 ** (exp + 1)


def ascii_chart(shares: list[float | None], first_ts: int, last_ts: int, date_fmt: str) -> list[str]:
    """Render a column chart of shares (fractions), one column per bucket.

    first_ts and last_ts label the left and right ends of the x axis."""
    top = nice_ceiling(max((s for s in shares if s is not None), default=0))
    width = max(1, min(6, CHART_WIDTH // len(shares)))
    labels = {CHART_HEIGHT: top, CHART_HEIGHT // 2: top / 2}
    pad = max(len(f"{v:.2%}") for v in (top, 0.0))
    lines = []
    for row in range(CHART_HEIGHT, 0, -1):
        # A cell is filled when the value reaches the middle of the cell.
        threshold = top * (row - 0.5) / CHART_HEIGHT
        cells = "".join(
            ("#" * (width - 1) + (" " if width > 1 else "#")) if s is not None and s >= threshold
            else " " * width
            for s in shares)
        label = f"{labels[row]:.2%}" if row in labels else ""
        lines.append(f"{label:>{pad}} |{cells.rstrip()}")
    lines.append(f"{0:>{pad}.2%} +{'-' * (len(shares) * width)}")
    axis_width = len(shares) * width
    left, right = fmt_time(first_ts, date_fmt), fmt_time(last_ts, date_fmt)
    gap = axis_width - len(left) - len(right)
    lines.append(" " * (pad + 2) + left + (" " * gap + right if gap >= 2 else ""))
    return lines


class Progress:
    def __init__(self, total: int):
        self.total, self.done = total, 0
        self.start = self.last = time.time()
        self.lock = threading.Lock()
        self.tty = sys.stderr.isatty()

    def add(self, n: int):
        with self.lock:
            self.done += n
            now = time.time()
            if now - self.last < (1 if self.tty else 15) and self.done < self.total:
                return
            self.last = now
            rate = self.done / max(now - self.start, 1e-9)
            eta = (self.total - self.done) / rate if rate else 0
            msg = (f"fetched {self.done:,}/{self.total:,} slots ({self.done / self.total:.0%}), "
                   f"{rate:,.0f} slots/s, ETA {eta:,.0f}s")
            print(("\r" + msg + "   ") if self.tty else msg, end="" if self.tty else "\n",
                  file=sys.stderr, flush=True)

    def finish(self):
        if self.tty:
            print(file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(
        description="Estimate execution client share of proposed blocks from block graffiti.")
    ap.add_argument("--range", dest="range_s", type=parse_duration, default="30d",
                    help="time period ending at the latest finalized slot, e.g. 30d, 12h, 2w (default: 30d)")
    ap.add_argument("--beacon-url", default="http://localhost:5052",
                    help="beacon node API URL (default: http://localhost:5052)")
    ap.add_argument("--client", default="ethrex",
                    help="execution client to chart and estimate the stake of (default: ethrex)")
    ap.add_argument("--bucket", type=parse_duration, default=None,
                    help="time bucket for the chart, e.g. 6h, 1d (default: picked from the range)")
    ap.add_argument("--concurrency", type=int, default=16,
                    help="parallel block requests (default: 16)")
    args = ap.parse_args()

    target = next((c for c, n in EL_CODES.items() if args.client.lower() in (c.lower(), n.lower())), None)
    if target is None:
        ap.error(f"unknown client {args.client!r}; known: {', '.join(EL_CODES.values())}")

    beacon = BeaconClient(args.beacon_url)
    genesis_time = int(beacon.get_json("/eth/v1/beacon/genesis")["data"]["genesis_time"])
    seconds_per_slot = int(beacon.get_json("/eth/v1/config/spec")["data"]["SECONDS_PER_SLOT"])
    end = int(beacon.get_json("/eth/v1/beacon/headers/finalized")["data"]["header"]["message"]["slot"])
    start = max(0, end - args.range_s // seconds_per_slot + 1)

    # Blinded blocks skip the transaction list; fall back to full blocks if unsupported.
    probe = beacon.request("/eth/v1/beacon/blinded_blocks/head")
    probe.read()
    block_path = "/eth/v1/beacon/blinded_blocks/{}" if probe.status == 200 else "/eth/v2/beacon/blocks/{}"

    print(f"Fetching slots {start:,}..{end:,} from {args.beacon_url}", file=sys.stderr)
    progress = Progress(end - start + 1)

    def fetch_chunk(slots):
        out = []
        for slot in slots:
            block = beacon.get_json(block_path.format(slot))
            if block is None:
                out.append(None)  # empty slot
            else:
                out.append(el_code_from_graffiti(block["data"]["message"]["body"]["graffiti"]) or "")
        progress.add(len(slots))
        return out

    chunks = [range(s, min(s + CHUNK, end + 1)) for s in range(start, end + 1, CHUNK)]
    results: list[str | None] = []  # per slot: None = empty, "" = untagged, else EL code
    with ThreadPoolExecutor(max_workers=args.concurrency) as ex:
        for part in ex.map(fetch_chunk, chunks):
            results.extend(part)
    progress.finish()

    first = next((i for i, r in enumerate(results) if r is not None), None)
    if first is None:
        sys.exit(f"error: the beacon node returned no blocks for slots {start}..{end}")
    if first >= MAX_EMPTY_RUN:
        print(f"warning: the beacon node has no blocks before slot {start + first:,} "
              f"(it was probably checkpoint-synced); using slots {start + first:,}..{end:,} only",
              file=sys.stderr)
        start, results = start + first, results[first:]

    print("Fetching active validator balances", file=sys.stderr)
    try:
        stake = total_active_stake(beacon)
    except (OSError, http.client.HTTPException, RuntimeError) as e:
        print(f"warning: could not fetch validator balances, skipping the stake estimate: {e}", file=sys.stderr)
        stake = None
    report(results, start, end, genesis_time, seconds_per_slot, target, args.bucket, stake)


def report(results: list[str | None], start: int, end: int, genesis_time: int, seconds_per_slot: int,
           target: str, bucket_s: int | None, stake: tuple[int, int] | None):
    """Print the summary for per-slot results covering slots start..end.

    stake is (total active effective balance in gwei, active validator count), or None."""
    blocks = [r for r in results if r is not None]
    total = len(blocks)
    counts = Counter(r for r in blocks if r)
    tagged = sum(counts.values())
    counts.setdefault(target, 0)
    name = EL_CODES[target]

    print(f"Range: {fmt_time(genesis_time + start * seconds_per_slot)} .. "
          f"{fmt_time(genesis_time + end * seconds_per_slot)} "
          f"(slots {start}..{end}, {len(results) - total:,} empty)")
    print(f"Total blocks: {total:,}")
    print(f"Tagged blocks: {tagged:,} ({tagged / total:.1%})")
    print()
    print("Client composition of total blocks:")
    for code, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"- {EL_CODES[code]}: {n / total:.2%}")
    print(f"- Untagged: {(total - tagged) / total:.2%}")

    # Share of the target client per time bucket. Buckets are aligned to the
    # end of the range, so only the oldest one can be partial.
    span = (end - start + 1) * seconds_per_slot
    bucket_s = bucket_s or next((b for b in BUCKET_SIZES if span / b <= CHART_MAX_BUCKETS), BUCKET_SIZES[-1])
    bucket_slots = max(1, bucket_s // seconds_per_slot)
    n_buckets = math.ceil(len(results) / bucket_slots)
    tot_b, hit_b = [0] * n_buckets, [0] * n_buckets
    for i, r in enumerate(results):
        if r is None:
            continue
        b = n_buckets - 1 - (len(results) - 1 - i) // bucket_slots
        tot_b[b] += 1
        hit_b[b] += r == target
    shares = [h / t if t else None for h, t in zip(hit_b, tot_b)]
    date_fmt = "%Y-%m-%d" if bucket_s % 86400 == 0 else "%m-%d %H:%M"
    print()
    print(f"{name} share of total blocks per {fmt_duration(bucket_s)} "
          f"(first bucket {shares[0] or 0:.2%}, last bucket {shares[-1] or 0:.2%}):")
    for line in ascii_chart(shares, genesis_time + start * seconds_per_slot,
                            genesis_time + (end + 1) * seconds_per_slot, date_fmt):
        print(line)

    if stake is not None:
        stake_eth = stake[0] / 1e9
        print()
        print(f"Total active stake: {stake_eth:,.0f} ETH ({stake[1]:,} validators)")
        print(f"Estimated stake running on {name}: {stake_eth * counts[target] / total:,.0f} ETH")


if __name__ == "__main__":
    main()

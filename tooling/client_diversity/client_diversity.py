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
                        [--client ethrex] [--bucket DURATION] [--concurrency 16] [--no-stake]

Pure stdlib; no third-party dependencies.
"""
from __future__ import annotations

import argparse
import base64
import datetime as dt
import http.client
import json
import math
import os
import re
import socket
import ssl
import sys
import threading
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from urllib.parse import unquote, urlsplit

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
    "PR",  # Prysm v7.1.4 put PR instead of PM in the graffiti (fixed in a later release)
}

# Tag forms used by consensus clients (the layout is not part of the
# execution-apis spec): EL code + CL code (4 chars), each followed by 2 or 4
# lowercase hex chars of its commit (8 or 12 chars). Local Prysm builds put
# "Local" in place of their commit, seen on mainnet as `GE13d8PMLoca`. The
# `(?![A-Za-z0-9])` makes the tag end at a word boundary; where it starts is
# checked in `el_code_from_graffiti`, since Prysm glues it to a full graffiti.
TAG_RE = re.compile(
    r"([A-Z]{2})(?:[0-9a-f]{4}([A-Z]{2})(?:[0-9a-f]{4}|Loca)|[0-9a-f]{2}([A-Z]{2})(?:[0-9a-f]{2}|Lo)|([A-Z]{2}))"
    r"(?![A-Za-z0-9])")
# With 3 bytes free, Lighthouse, Teku and Prysm append just the EL code after a space.
EL_ONLY_RE = re.compile(r"(?:^|[^A-Za-z0-9])([A-Z]{2})\Z")
# Clients only glue the tag to the user graffiti, or shrink it to the EL code,
# when the graffiti is nearly full. Requiring that keeps words like "ANGELS"
# (glued GE+LS) or "... TE" in short graffiti from counting as tags.
NEARLY_FULL = 30
GRAFFITI_BYTES = 32
EFFECTIVE_BALANCE_RE = re.compile(rb'"effective_balance"\s*:\s*"(\d+)"')

# A run of empty slots this long (2 epochs) means the node is missing blocks,
# e.g. after checkpoint sync without full backfill, or a major network outage.
MAX_EMPTY_RUN = 64
CHUNK = 64
RETRIES = 5
CHART_HEIGHT = 10
CHART_WIDTH = 60
CHART_MAX_BUCKETS = 40  # automatic bucket choice
CHART_MAX_BUCKETS_EXPLICIT = 200  # --bucket
UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}
BUCKET_SIZES = [3600 * h for h in (1, 2, 3, 6, 12)] + [86400 * d for d in (1, 2, 3, 7, 14, 30)]
EMPTY, ERROR = None, "!"  # per-slot markers in the results list, next to "" (untagged) and EL codes


def el_code_from_graffiti(graffiti_hex: str) -> str | None:
    """Return the execution client code in a 0x-prefixed graffiti, if any.

    When several candidates appear, the one closest to the end wins: clients
    append the tag after the user graffiti."""
    try:
        raw = bytes.fromhex(graffiti_hex[2:]).rstrip(b"\x00")
    except ValueError:
        return None
    text = raw.decode("utf-8", "replace")
    full = len(raw) >= NEARLY_FULL
    best, best_end = None, -1
    for m in TAG_RE.finditer(text):
        # Standalone word, or glued to the end of a nearly full graffiti.
        glued = m.start() > 0 and text[m.start() - 1].isascii() and text[m.start() - 1].isalnum()
        if glued and not (full and m.end() == len(text)):
            continue
        el, cl = m.group(1), m.group(2) or m.group(3) or m.group(4)
        if el in EL_CODES and cl in CL_CODES and m.end() > best_end:
            best, best_end = el, m.end()
    m = EL_ONLY_RE.search(text) if full else None
    if m and m.group(1) in EL_CODES and m.end() > best_end:
        best, best_end = m.group(1), m.end()
    # With exactly 2 bytes free, Prysm glues the EL code to the user graffiti
    # (`...manuallyGE`). Skip it after a capital letter, as in "CHANGE".
    if best_end < len(text) and len(raw) == GRAFFITI_BYTES and len(text) >= 3 \
            and text[-2:] in EL_CODES and not ("A" <= text[-3] <= "Z"):
        best = text[-2:]
    return best


class HTTPError(Exception):
    def __init__(self, path: str, status: int, body: bytes):
        super().__init__(f"GET {path}: HTTP {status}: {body[:200]!r}")
        self.status = status


class Aborted(Exception):
    pass


def retryable(e: Exception) -> bool:
    if isinstance(e, HTTPError):
        return e.status == 429 or (e.status >= 500 and e.status != 501)
    if isinstance(e, (socket.gaierror, ssl.SSLCertVerificationError)):
        return False  # DNS and certificate errors won't fix themselves
    return isinstance(e, (OSError, http.client.HTTPException))


class BeaconClient:
    """Minimal beacon API client with one keep-alive connection per thread.

    Setting `abort` makes every pending and future get() fail fast."""

    def __init__(self, url: str):
        u = urlsplit(url)
        if u.scheme not in ("http", "https") or not u.hostname:
            raise ValueError(f"unsupported beacon URL: {url}")
        self.scheme, self.host = u.scheme, u.hostname
        # Explicit port: http.client would re-parse an IPv6 host like "::1" without one.
        self.port = u.port or (443 if u.scheme == "https" else 80)
        self.prefix = u.path.rstrip("/")
        self.query = u.query  # e.g. an API key
        self.headers = {"Accept": "application/json"}
        if u.username is not None:
            creds = f"{unquote(u.username)}:{unquote(u.password or '')}".encode()
            self.headers["Authorization"] = "Basic " + base64.b64encode(creds).decode()
        self.local = threading.local()
        self.abort = threading.Event()
        self.conns: list[http.client.HTTPConnection] = []
        self.conns_lock = threading.Lock()

    def _conn(self, timeout: float) -> http.client.HTTPConnection:
        c = getattr(self.local, "conn", None)
        if c is not None and c.timeout != timeout:
            self.drop_conn()
            c = None
        if c is None:
            cls = http.client.HTTPSConnection if self.scheme == "https" else http.client.HTTPConnection
            c = self.local.conn = cls(self.host, self.port, timeout=timeout)
            with self.conns_lock:
                self.conns.append(c)
        return c

    def close(self):
        """Close the connections of every thread."""
        with self.conns_lock:
            for c in self.conns:
                c.close()
            self.conns.clear()

    def drop_conn(self):
        c = getattr(self.local, "conn", None)
        if c is not None:
            c.close()
            with self.conns_lock:
                if c in self.conns:
                    self.conns.remove(c)
        self.local.conn = None

    def request(self, path: str, timeout: float = 60) -> http.client.HTTPResponse:
        """Send one GET and return the response with its body unread (no retries).

        The caller must read the body, or call drop_conn(), before the next request."""
        if self.query:
            path += ("&" if "?" in path else "?") + self.query
        try:
            c = self._conn(timeout)
            c.request("GET", self.prefix + path, headers=self.headers)
            return c.getresponse()
        except BaseException:
            self.drop_conn()
            raise

    def get(self, path: str) -> tuple[int, bytes]:
        """GET path, retrying connection errors, 5xx and 429. Returns (status, body)."""
        for attempt in range(RETRIES + 1):
            if self.abort.is_set():
                raise Aborted()
            try:
                r = self.request(path)
                body = r.read()
                if r.status == 429 or r.status >= 500:
                    self.drop_conn()
                    raise HTTPError(path, r.status, body)
                return r.status, body
            except Exception as e:
                self.drop_conn()
                if attempt == RETRIES or not retryable(e):
                    raise
                time.sleep(0.5 * (attempt + 1))

    def get_json(self, path: str):
        """GET path as JSON. Returns None on 404, raises on other errors."""
        status, body = self.get(path)
        if status == 404:
            return None
        if status != 200:
            raise HTTPError(path, status, body)
        return json.loads(body)


def parse_duration(s: str) -> int:
    """Parse a duration like 30d, 12h, 2w, 90m into seconds."""
    m = re.fullmatch(r"(\d+)\s*([smhdw])", s.strip())
    if not m or int(m.group(1)) == 0:
        raise argparse.ArgumentTypeError(f"invalid duration {s!r}, expected e.g. 30d, 12h, 2w")
    return int(m.group(1)) * UNITS[m.group(2)]


def positive_int(s: str) -> int:
    try:
        n = int(s)
    except ValueError:
        n = 0
    if n <= 0:
        raise argparse.ArgumentTypeError(f"expected a positive integer, got {s!r}")
    return n


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
    instead of being parsed as one JSON document."""
    path = "/eth/v1/beacon/states/finalized/validators?status=active"
    beacon.drop_conn()  # the connection sat idle during the scan; a proxy may have closed it
    for attempt in range(3):
        try:
            return _sum_effective_balances(beacon, path)
        except Exception as e:
            beacon.drop_conn()
            if attempt == 2 or not (retryable(e) or isinstance(e, IncompleteResponse)):
                raise
            time.sleep(2 * (attempt + 1))


class IncompleteResponse(Exception):
    pass


def _sum_effective_balances(beacon: BeaconClient, path: str) -> tuple[int, int]:
    # Some nodes build the whole response before sending the first byte.
    r = beacon.request(path, timeout=300)
    if r.status != 200:
        raise HTTPError(path, r.status, r.read(200))
    expected = r.getheader("Content-Length")
    total = count = size = depth = 0
    buf = b""
    while True:
        chunk = r.read(1 << 20)
        if not chunk:
            break
        size += len(chunk)
        # Nesting depth of the JSON document; its strings (hex, statuses) hold no brackets.
        depth += chunk.count(b"{") + chunk.count(b"[") - chunk.count(b"}") - chunk.count(b"]")
        buf += chunk
        last = 0
        for m in EFFECTIVE_BALANCE_RE.finditer(buf):
            total += int(m.group(1))
            count += 1
            last = m.end()
        buf = buf[max(last, len(buf) - 64):]
    # read() returns b"" instead of raising when the server closes early, so
    # check that the whole document arrived.
    if (expected is not None and size != int(expected)) or depth != 0 or not buf.rstrip().endswith(b"}"):
        raise IncompleteResponse(f"validators response truncated after {size:,} bytes")
    if count == 0:
        raise IncompleteResponse("validators response has no active validators")
    return total, count


def fmt_share(x: float | None) -> str:
    """Format a fraction as a percentage with at least two significant digits."""
    if x is None:
        return "n/a"
    decimals = 2 if x == 0 else max(2, 1 - math.floor(math.log10(x * 100)))
    return f"{x:.{decimals}%}"


def nice_ceiling(x: float) -> float:
    """Smallest 'round' number (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 x 10^k) >= x > 0."""
    exp = math.floor(math.log10(x))
    for m in (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10):
        if m * 10 ** exp >= x * (1 - 1e-9):
            return m * 10 ** exp
    return 10 ** (exp + 1)


def ascii_chart(shares: list[float | None], first_ts: int, last_ts: int, date_fmt: str) -> list[str]:
    """Render a column chart of shares (fractions in (0, 1]), one column per bucket.

    At least one share must be positive. first_ts and last_ts label the left
    and right ends of the x axis."""
    top = nice_ceiling(max(s for s in shares if s is not None))
    # Enough decimals to print the middle label (e.g. 0.125%) exactly.
    decimals = next(d for d in range(2, 12) if abs(round(top * 50, d) - top * 50) < 1e-9 * top * 50)
    width = max(1, min(6, CHART_WIDTH // len(shares)))
    labels = {CHART_HEIGHT: top, CHART_HEIGHT // 2: top / 2}
    pad = len(f"{top:.{decimals}%}")
    lines = []
    for row in range(CHART_HEIGHT, 0, -1):
        # A cell is filled when the value reaches the middle of the cell.
        threshold = top * (row - 0.5) / CHART_HEIGHT
        cells = "".join(
            ("#" * (width - 1) + (" " if width > 1 else "#")) if s is not None and s >= threshold
            else " " * width
            for s in shares)
        label = f"{labels[row]:.{decimals}%}" if row in labels else ""
        lines.append(f"{label:>{pad}} |{cells.rstrip()}")
    lines.append(f"{0:>{pad}.{decimals}%} +{'-' * (len(shares) * width)}")
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


def empty_runs(results: list[str | None], min_len: int) -> list[tuple[int, int]]:
    """(offset, length) of every run of at least min_len empty slots."""
    runs, i = [], 0
    while i < len(results):
        if results[i] is EMPTY:
            j = i
            while j < len(results) and results[j] is EMPTY:
                j += 1
            if j - i >= min_len:
                runs.append((i, j - i))
            i = j
        else:
            i += 1
    return runs


def main():
    ap = argparse.ArgumentParser(
        description="Estimate execution client share of proposed blocks from block graffiti.")
    ap.add_argument("--range", dest="range_s", metavar="DURATION", type=parse_duration, default="30d",
                    help="time period ending at the latest finalized slot, e.g. 30d, 12h, 2w (default: 30d)")
    ap.add_argument("--beacon-url", default="http://localhost:5052",
                    help="beacon node API URL; may include credentials or a query string "
                         "(default: http://localhost:5052)")
    ap.add_argument("--client", default="ethrex",
                    help="execution client to chart and estimate the stake of, by name or two-letter code, "
                         "e.g. geth or GE (default: ethrex)")
    ap.add_argument("--bucket", metavar="DURATION", type=parse_duration, default=None,
                    help="time bucket for the chart, e.g. 6h, 1d (default: picked from the range)")
    ap.add_argument("--concurrency", type=positive_int, default=16,
                    help="parallel block requests (default: 16)")
    ap.add_argument("--no-stake", action="store_true",
                    help="skip the stake estimate, which downloads every active validator "
                         "(a few hundred MB on mainnet)")
    args = ap.parse_args()

    target = next((c for c, n in EL_CODES.items() if args.client.lower() in (c.lower(), n.lower())), None)
    if target is None:
        ap.error(f"unknown client {args.client!r}; use a name or code: "
                 + ", ".join(f"{n} ({c})" for c, n in EL_CODES.items()))
    try:
        beacon = BeaconClient(args.beacon_url)
    except ValueError as e:
        ap.error(str(e))
    try:
        run(args, ap, beacon, target)
    finally:
        beacon.close()


def run(args: argparse.Namespace, ap: argparse.ArgumentParser, beacon: BeaconClient, target: str):
    """Scan the range and print the report; `target` is the EL code to chart."""
    u = urlsplit(args.beacon_url)
    shown_url = f"{u.scheme}://{u.hostname}{':' + str(u.port) if u.port else ''}{u.path}"  # no credentials

    def fail(e: Exception):
        sys.exit(f"error: cannot query the beacon node at {shown_url} (see --beacon-url): "
                 f"{type(e).__name__}: {e}")

    def required(path: str):
        data = beacon.get_json(path)
        if data is None:
            raise RuntimeError(f"HTTP 404 for {path}; is this a beacon node API URL?")
        return data["data"]

    try:
        seconds_per_slot = int(required("/eth/v1/config/spec")["SECONDS_PER_SLOT"])
    except Exception as e:
        fail(e)
    if args.range_s < seconds_per_slot:
        ap.error(f"--range must be at least one slot ({seconds_per_slot}s)")
    range_slots = args.range_s // seconds_per_slot
    if args.bucket is not None:
        if args.bucket < seconds_per_slot:
            ap.error(f"--bucket must be at least one slot ({seconds_per_slot}s)")
        if math.ceil(range_slots / (args.bucket // seconds_per_slot)) > CHART_MAX_BUCKETS_EXPLICIT:
            ap.error(f"--bucket {fmt_duration(args.bucket)} gives more than {CHART_MAX_BUCKETS_EXPLICIT} "
                     f"chart columns for --range {fmt_duration(args.range_s)}")

    try:
        genesis_time = int(required("/eth/v1/beacon/genesis")["genesis_time"])
        end = int(required("/eth/v1/beacon/headers/finalized")["header"]["message"]["slot"])
        # Blinded blocks skip the transaction list, but some nodes don't serve
        # them. Probe with the finalized slot, which has a block.
        block_path = None
        for path in ("/eth/v1/beacon/blinded_blocks/{}", "/eth/v2/beacon/blocks/{}"):
            try:
                status, _ = beacon.get(path.format(end))
            except HTTPError:
                continue
            if status == 200:
                block_path = path
                break
    except Exception as e:
        fail(e)
    if block_path is None:
        sys.exit(f"error: the beacon node did not return the finalized block at slot {end}")
    start = max(0, end - range_slots + 1)

    print(f"Fetching slots {start:,}..{end:,} from {block_path.format('{slot}')}", file=sys.stderr)
    if "blinded" not in block_path:
        print("note: the node does not serve blinded blocks; full blocks are much larger, "
              "so this is slower", file=sys.stderr)
    progress = Progress(end - start + 1)
    lock = threading.Lock()
    failures = {"first": None, "run": 0}
    # Single failed slots are skipped, but this many in a row means the node is gone.
    max_failed_run = max(MAX_EMPTY_RUN, 4 * args.concurrency)

    def fetch_slot(slot: int) -> str | None:
        block = beacon.get_json(block_path.format(slot))
        if block is None:
            return EMPTY
        message = block["data"]["message"]
        if int(message["slot"]) != slot:
            raise ValueError(f"asked for slot {slot}, got slot {message['slot']}")
        return el_code_from_graffiti(message["body"]["graffiti"]) or ""

    def fetch_chunk(slots):
        out = []
        for slot in slots:
            try:
                out.append(fetch_slot(slot))
                with lock:
                    failures["run"] = 0
            except Exception as e:  # skip the slot rather than abort a long scan
                out.append(ERROR)
                with lock:
                    failures["first"] = failures["first"] or f"slot {slot}: {type(e).__name__}: {e}"
                    failures["run"] += 1
                    if failures["run"] >= max_failed_run:
                        beacon.abort.set()
        progress.add(len(slots))
        return out

    # Small enough chunks that short ranges still use every worker.
    chunk = max(1, min(CHUNK, math.ceil((end - start + 1) / (args.concurrency * 4))))
    chunks = [range(s, min(s + chunk, end + 1)) for s in range(start, end + 1, chunk)]
    results: list[str | None] = []
    ex = ThreadPoolExecutor(max_workers=args.concurrency)
    try:
        for part in ex.map(fetch_chunk, chunks):
            results.extend(part)
    except KeyboardInterrupt:
        # Workers may be stuck in requests and retries; don't wait for them.
        sys.stderr.write("\n")
        sys.stderr.flush()
        os._exit(130)
    ex.shutdown()
    progress.finish()
    if beacon.abort.is_set():
        sys.exit(f"error: the beacon node stopped answering ({max_failed_run} failed slots in a row); "
                 f"e.g. {failures['first']}")

    errors = results.count(ERROR)
    if errors:
        print(f"warning: skipped {errors:,} slots that could not be fetched ({errors / len(results):.2%}); "
              f"e.g. {failures['first']}", file=sys.stderr)
    if all(r is EMPTY or r == ERROR for r in results):
        sys.exit(f"error: the beacon node returned no blocks for slots {start}..{end}")
    # Only a leading run of 404s means missing history; failed fetches are counted as skipped.
    first = next(i for i, r in enumerate(results) if r is not EMPTY)
    if first >= MAX_EMPTY_RUN:
        print(f"warning: the beacon node has no blocks before slot {start + first:,} "
              f"(it was probably checkpoint-synced); using slots {start + first:,}..{end:,} only",
              file=sys.stderr)
        start, results = start + first, results[first:]
    gaps = empty_runs(results, MAX_EMPTY_RUN)
    if gaps:
        print(f"warning: {sum(n for _, n in gaps):,} slots are in runs of {MAX_EMPTY_RUN}+ empty slots "
              f"(first at slots {start + gaps[0][0]:,}..{start + gaps[0][0] + gaps[0][1] - 1:,}); "
              f"the beacon node may be missing those blocks", file=sys.stderr)

    stake = None
    if not args.no_stake and target in results:  # no download when the estimate is 0 anyway
        print("Fetching active validator balances", file=sys.stderr)
        try:
            stake = total_active_stake(beacon)
        except Exception as e:
            print(f"warning: could not fetch validator balances, skipping the stake estimate: {e}",
                  file=sys.stderr)
    report(results, start, end, genesis_time, seconds_per_slot, target, args.bucket, stake,
           show_stake=not args.no_stake)


def report(results: list[str | None], start: int, end: int, genesis_time: int, seconds_per_slot: int,
           target: str, bucket_s: int | None, stake: tuple[int, int] | None, show_stake: bool = True):
    """Print the summary for per-slot results covering slots start..end.

    results holds one entry per slot: EMPTY, ERROR, "" (untagged) or an EL code.
    stake is (total active effective balance in gwei, active validator count), or None
    when it was not fetched."""
    blocks = [r for r in results if r is not EMPTY and r != ERROR]
    total = len(blocks)
    counts = Counter(r for r in blocks if r)
    tagged = sum(counts.values())
    counts.setdefault(target, 0)
    name = EL_CODES[target]
    empty, errors = results.count(EMPTY), results.count(ERROR)

    print(f"Range: {fmt_time(genesis_time + start * seconds_per_slot)} .. "
          f"{fmt_time(genesis_time + (end + 1) * seconds_per_slot)} "
          f"(slots {start:,}..{end:,}, {empty:,} empty" + (f", {errors:,} skipped" if errors else "") + ")")
    print(f"Total blocks: {total:,}")
    print(f"Tagged blocks: {tagged:,} ({tagged / total:.1%})")
    print()
    print("Client composition of total blocks:")
    for code, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"- {EL_CODES[code]}: {fmt_share(n / total)}")
    print(f"- Untagged: {fmt_share((total - tagged) / total)}")

    # Share of the target client per time bucket. Buckets are aligned to the
    # end of the range, so only the oldest one can be partial.
    span = (end - start + 1) * seconds_per_slot
    bucket_s = bucket_s or next((b for b in BUCKET_SIZES if span / b <= CHART_MAX_BUCKETS), BUCKET_SIZES[-1])
    bucket_slots = max(1, bucket_s // seconds_per_slot)
    bucket_s = bucket_slots * seconds_per_slot  # whole slots
    n_buckets = math.ceil(len(results) / bucket_slots)
    tot_b, hit_b = [0] * n_buckets, [0] * n_buckets
    for i, r in enumerate(results):
        if r is EMPTY or r == ERROR:
            continue
        b = n_buckets - 1 - (len(results) - 1 - i) // bucket_slots
        tot_b[b] += 1
        hit_b[b] += r == target
    shares = [h / t if t else None for h, t in zip(hit_b, tot_b)]
    print()
    if counts[target] == 0:
        print(f"No {name} blocks in this range.")
    else:
        partial = " (partial)" if len(results) % bucket_slots else ""
        print(f"{name} share of total blocks per {fmt_duration(bucket_s)} "
              f"(first bucket{partial} {fmt_share(shares[0])}, last bucket {fmt_share(shares[-1])}):")
        date_fmt = "%Y-%m-%d" if bucket_s % 86400 == 0 else "%m-%d %H:%M"
        for line in ascii_chart(shares, genesis_time + start * seconds_per_slot,
                                genesis_time + (end + 1) * seconds_per_slot, date_fmt):
            print(line)

    if not show_stake:
        return
    if counts[target] == 0:
        print()
        print(f"Estimated stake running on {name}: 0 ETH (no {name} blocks)")
    elif stake is not None:
        stake_eth = stake[0] / 1e9
        print()
        print(f"Total active stake: {stake_eth:,.0f} ETH ({stake[1]:,} validators)")
        print(f"Estimated stake running on {name}: {stake_eth * counts[target] / total:,.0f} ETH "
              f"(lower bound; {counts[target]:,} of {total:,} blocks)")
        print(f"  ({stake_eth * counts[target] / tagged:,.0f} ETH if untagged blocks have the same "
              f"client mix as tagged ones)")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)

#!/usr/bin/env python3
"""Hegotá devnet faucet.

Dispenses test ETH over `POST /api/claim {"address": "0x..."}`, estimating gas so
it works for addresses that do not exist yet (EIP-8038 prices account creation
well above the historical 21000, which is what broke the off-the-shelf faucet).

Rate limiting, and the reasoning behind each rule, is in DESIGN.md.

Config (env):
  RPC_URL                  execution RPC                        (required)
  PRIVATE_KEY              funding key, dedicated to this        (required)
  AMOUNT_ETH               dispensed per claim                   (default 1)
  PER_IP_MINUTES           per-IP-bucket window                  (default 60)
  PER_ADDR_MINUTES         per-recipient window                  (default 60)
  GLOBAL_PER_HOUR          cap on total claims per hour          (default 100)
  MIN_RESERVE_ETH          refuse below this balance             (default 100)
  MAX_RECIPIENT_ETH        refuse recipients richer than this    (default 10)
  TRUSTED_PROXIES          CIDRs whose X-Forwarded-For is honored
                           (default loopback + private ranges)
  MAX_CONCURRENT           in-flight requests before shedding    (default 32)
  BIND_ADDR                                                      (default 0.0.0.0)
  PORT                                                           (default 8080)
  PUBLIC_RPC_URL           shown on the landing page             (optional)
  EXPLORER_URL             shown on the landing page             (optional)

Deployment note: only `TRUSTED_PROXIES` peers may set the client IP. Publish the
port to loopback on the host and put the reverse proxy in front; a directly
reachable port lets callers choose their own rate-limit bucket.
"""
import ipaddress
import json
import os
import pathlib
import threading
import time
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import error as urlerror
from urllib import request as urlrequest

from eth_account import Account
from eth_utils import is_hex_address, to_checksum_address

RPC_URL = os.environ["RPC_URL"]
ACCT = Account.from_key(os.environ["PRIVATE_KEY"])
AMOUNT_WEI = int(float(os.environ.get("AMOUNT_ETH", "1")) * 10**18)
PER_IP_WINDOW = int(os.environ.get("PER_IP_MINUTES", "60")) * 60
PER_ADDR_WINDOW = int(os.environ.get("PER_ADDR_MINUTES", "60")) * 60
GLOBAL_PER_HOUR = int(os.environ.get("GLOBAL_PER_HOUR", "100"))
MIN_RESERVE_WEI = int(float(os.environ.get("MIN_RESERVE_ETH", "100")) * 10**18)
MAX_RECIPIENT_WEI = int(float(os.environ.get("MAX_RECIPIENT_ETH", "10")) * 10**18)
MAX_CONCURRENT = int(os.environ.get("MAX_CONCURRENT", "32"))
BIND_ADDR = os.environ.get("BIND_ADDR", "0.0.0.0")
PORT = int(os.environ.get("PORT", "8080"))
PUBLIC_RPC_URL = os.environ.get("PUBLIC_RPC_URL", "")
EXPLORER_URL = os.environ.get("EXPLORER_URL", "")

# A reverse proxy always reaches us from loopback or a private address, and a
# public client cannot spoof a private source address. Anything else must not be
# allowed to choose its own rate-limit bucket.
TRUSTED_PROXIES = [
    ipaddress.ip_network(c.strip())
    for c in os.environ.get(
        "TRUSTED_PROXIES", "127.0.0.0/8,::1/128,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7"
    ).split(",")
    if c.strip()
]

MAX_BODY = 1024
MAX_TRACKED = 100_000
GAS_MARGIN_NUM, GAS_MARGIN_DEN = 5, 4  # 1.25x
GAS_FALLBACK = 300_000
FEE_MULTIPLIER = 4  # headroom so a rising base fee cannot strand a claim
RECEIPT_POLL_SECONDS = 20


class RpcError(RuntimeError):
    """The node answered with a JSON-RPC error, i.e. it definitely saw the call."""


def rpc(method, params, timeout=15):
    """Call the node. `RpcError` means the node replied; anything else means the
    outcome is unknown and callers must assume the call may have taken effect."""
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    req = urlrequest.Request(RPC_URL, data=body.encode(),
                             headers={"content-type": "application/json"})
    try:
        with urlrequest.urlopen(req, timeout=timeout) as resp:
            out = json.loads(resp.read())
    except urlerror.HTTPError as err:
        raise ConnectionError(f"rpc http {err.code}") from err
    if "error" in out:
        raise RpcError(out["error"].get("message", "rpc error"))
    return out["result"]


class Limiter:
    """Windowed single-claim buckets plus a global hourly cap.

    Entries are pruned on access and the map is hard-capped, so a flood of
    distinct keys cannot grow it without bound.
    """

    def __init__(self):
        self.lock = threading.Lock()
        self.seen: dict[str, float] = {}
        self.global_hits: deque[float] = deque()

    def _prune(self, now):
        for key in [k for k, exp in self.seen.items() if exp <= now]:
            del self.seen[key]
        if len(self.seen) > MAX_TRACKED:
            oldest = sorted(self.seen.items(), key=lambda kv: kv[1])[: len(self.seen) // 4]
            for key, _ in oldest:
                del self.seen[key]
        while self.global_hits and now - self.global_hits[0] > 3600:
            self.global_hits.popleft()

    def check(self, keys_with_windows):
        """Reserve every key at once, or reject without consuming any of them.

        Returns `(denial_or_None, token)`; pass the token to `release` to undo.
        """
        now = time.time()
        with self.lock:
            self._prune(now)
            if len(self.global_hits) >= GLOBAL_PER_HOUR:
                return "the faucet is rate limited globally, try again later", None
            for key, _ in keys_with_windows:
                exp = self.seen.get(key)
                if exp and exp > now:
                    wait = int(exp - now) // 60 + 1
                    return f"rate limited, try again in {wait} minute(s)", None
            for key, window in keys_with_windows:
                self.seen[key] = now + window
            self.global_hits.append(now)
            return None, now

    def release(self, keys_with_windows, token):
        """Give the windows back when the claim demonstrably did not happen."""
        with self.lock:
            for key, _ in keys_with_windows:
                self.seen.pop(key, None)
            if token is not None:
                try:
                    self.global_hits.remove(token)
                except ValueError:
                    pass


class Sender:
    """Owns the nonce. Only signing and submission are serialized, so a slow
    node cannot head-of-line block unrelated requests for longer than one send.
    """

    def __init__(self):
        self.nonce_lock = threading.Lock()
        self.inflight_lock = threading.Lock()
        self.nonce = None
        self.chain_id = None
        self.inflight: set[str] = set()

    def claim_inflight(self, addr):
        with self.inflight_lock:
            if addr in self.inflight:
                return False
            self.inflight.add(addr)
            return True

    def drop_inflight(self, addr):
        with self.inflight_lock:
            self.inflight.discard(addr)

    def invalidate_nonce(self):
        with self.nonce_lock:
            self.nonce = None

    def _fees(self):
        head = rpc("eth_getBlockByNumber", ["latest", False])
        base_fee = int(head.get("baseFeePerGas", "0x0"), 16)
        tip = 10**9
        return tip, base_fee * FEE_MULTIPLIER + tip

    def _gas_for(self, to):
        """Estimated gas, or None when the node says the call would fail."""
        try:
            est = int(rpc("eth_estimateGas", [
                {"from": ACCT.address, "to": to, "value": hex(AMOUNT_WEI)}]), 16)
            return est * GAS_MARGIN_NUM // GAS_MARGIN_DEN
        except RpcError:
            # The node executed it and it reverted/halted: sending would only
            # burn the fallback gas.
            return None
        except Exception:
            # Transport failure: the estimate is unknown, not known-bad.
            return GAS_FALLBACK

    def send(self, to):
        """Sign and submit. Raises `RpcError` when the node rejected the send
        (nothing happened) or `ConnectionError` when the outcome is unknown."""
        if self.chain_id is None:
            self.chain_id = int(rpc("eth_chainId", []), 16)
        gas = self._gas_for(to)
        if gas is None:
            raise RpcError("the node reports this transfer would fail")
        tip, max_fee = self._fees()

        with self.nonce_lock:
            if self.nonce is None:
                # `latest`, never `pending`: pending counts queued transactions,
                # so resyncing off it perpetuates a nonce gap instead of closing
                # it, and one gap would wedge the faucet permanently.
                self.nonce = int(rpc("eth_getTransactionCount", [ACCT.address, "latest"]), 16)
            for attempt in (1, 2):
                tx = {
                    "type": 2, "chainId": self.chain_id, "nonce": self.nonce,
                    "to": to, "value": AMOUNT_WEI, "gas": gas,
                    "maxPriorityFeePerGas": tip, "maxFeePerGas": max_fee, "accessList": [],
                }
                signed = ACCT.sign_transaction(tx)
                try:
                    tx_hash = rpc("eth_sendRawTransaction",
                                  ["0x" + signed.raw_transaction.hex()])
                except RpcError as err:
                    msg = str(err).lower()
                    if attempt == 1 and ("nonce" in msg or "already known" in msg):
                        self.nonce = int(
                            rpc("eth_getTransactionCount", [ACCT.address, "latest"]), 16)
                        continue
                    raise
                except Exception:
                    # Unknown outcome: the node may hold this transaction. Force a
                    # resync so the next claim cannot rebuild the same nonce.
                    self.nonce = None
                    raise
                self.nonce += 1
                return tx_hash
            raise RpcError("could not obtain a usable nonce")


LIMITER = Limiter()
SENDER = Sender()
SLOTS = threading.BoundedSemaphore(MAX_CONCURRENT)


def render_page():
    path = pathlib.Path(__file__).with_name("page.html")
    try:
        html = path.read_text()
    except OSError:
        return b"<h1>Hegota devnet faucet</h1><p>POST /api/claim {\"address\": \"0x...\"}</p>"
    rpc_row = f"<dt>RPC</dt><dd>{PUBLIC_RPC_URL}</dd>" if PUBLIC_RPC_URL else ""
    explorer_row = (f'<dt>Explorer</dt><dd><a href="{EXPLORER_URL}">{EXPLORER_URL}</a></dd>'
                    if EXPLORER_URL else "")
    chain_id = "unknown"
    try:
        raw = int(rpc("eth_chainId", [], timeout=5), 16)
        chain_id = f"{raw} ({hex(raw)})"
    except Exception:
        pass
    amount = f"{AMOUNT_WEI / 10**18:g}"
    return (html
            .replace("{{CHAIN_ID}}", chain_id)
            .replace("{{AMOUNT}}", amount)
            .replace("{{RPC}}", rpc_row)
            .replace("{{EXPLORER}}", explorer_row)).encode()


PAGE = render_page()


def load_guide():
    """The EIP guide, served as a static page. Read once at startup like PAGE.

    Returns None when the file is absent so an older image, or a build that
    omitted it, keeps serving the faucet instead of failing to boot.
    """
    try:
        return pathlib.Path(__file__).with_name("eips.html").read_bytes()
    except OSError:
        return None


GUIDE = load_guide()


def client_bucket(handler):
    """Rate-limit bucket for the caller.

    `X-Forwarded-For` is only consulted when the peer is a trusted proxy, since
    otherwise a caller picks its own bucket (and can burn someone else's). Each
    proxy appends the address it saw, so with N trusted hops the client is the
    Nth entry from the right; entries further left are caller-supplied.
    """
    peer_text = handler.client_address[0]
    try:
        peer = ipaddress.ip_address(peer_text)
    except ValueError:
        peer = None
    trusted = peer is not None and any(peer in net for net in TRUSTED_PROXIES)

    candidate = peer_text
    if trusted:
        # Repeated header lines must all be considered: proxies that add a new
        # line rather than merging would otherwise let the first, caller-supplied
        # line win.
        merged = ", ".join(handler.headers.get_all("X-Forwarded-For") or [])
        parts = [p.strip() for p in merged.split(",") if p.strip()]
        if parts:
            candidate = parts[-1]

    return bucket_for(candidate, fallback=peer_text)


def bucket_for(text, fallback):
    """Normalize an address into a bucket key, collapsing IPv6 to its /64."""
    cleaned = text.strip()
    if cleaned.startswith("["):  # [2001:db8::1]:443
        cleaned = cleaned[1:].split("]", 1)[0]
    elif cleaned.count(":") == 1:  # 1.2.3.4:5678
        cleaned = cleaned.split(":", 1)[0]
    try:
        ip = ipaddress.ip_address(cleaned)
    except ValueError:
        try:
            ip = ipaddress.ip_address(fallback)
        except ValueError:
            return "ip:unknown"
    # An IPv4-mapped address reports version 6 and a /64 of `::`, which would put
    # every caller in one bucket.
    if ip.version == 6 and ip.ipv4_mapped:
        ip = ip.ipv4_mapped
    if ip.version == 6:
        return f"ip6:{ipaddress.ip_network(f'{ip}/64', strict=False).network_address}"
    return f"ip4:{ip}"


class Handler(BaseHTTPRequestHandler):
    server_version = "hegota-faucet"
    protocol_version = "HTTP/1.1"
    timeout = 10  # without this a half-open connection is held forever

    def log_message(self, fmt, *args):
        # The peer is the proxy, so log the computed bucket too or abuse cannot
        # be attributed. Never log request bodies.
        stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        print(f"{stamp} {client_bucket(self)} {fmt % args}", flush=True)

    def _reply(self, code, payload):
        body = json.dumps(payload).encode()
        if code >= 400:
            # The body of a rejected request is still in the socket; keeping the
            # connection would desync it and hand the next request the leftovers.
            self.close_connection = True
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _path(self):
        return self.path.split("?", 1)[0].rstrip("/")

    def do_GET(self):
        path = self._path()
        if path in ("", "/index.html"):
            self.send_response(200)
            self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(PAGE)))
            self.end_headers()
            self.wfile.write(PAGE)
        elif path in ("/eips", "/eips.html") and GUIDE is not None:
            self.send_response(200)
            self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(GUIDE)))
            self.end_headers()
            self.wfile.write(GUIDE)
        elif path == "/healthz":
            self._reply(200, {"ok": True, "faucet": ACCT.address})
        else:
            self._reply(404, {"msg": "not found"})

    def do_POST(self):
        if not SLOTS.acquire(blocking=False):
            return self._reply(503, {"msg": "busy, try again shortly"})
        try:
            self._handle_post()
        finally:
            SLOTS.release()

    def _handle_post(self):
        if self._path() != "/api/claim":
            return self._reply(404, {"msg": "not found"})

        # A cross-site form POST cannot set either of these, so requiring them
        # stops a hostile page spending its visitors' quotas.
        ctype = (self.headers.get("content-type") or "").split(";")[0].strip().lower()
        if ctype != "application/json":
            return self._reply(415, {"msg": "content-type must be application/json"})
        origin = self.headers.get("Origin")
        if origin and not self._same_origin(origin):
            return self._reply(403, {"msg": "cross-origin requests are not accepted"})

        try:
            length = int(self.headers.get("content-length") or 0)
        except ValueError:
            return self._reply(400, {"msg": "bad content-length"})
        if length <= 0:
            return self._reply(400, {"msg": "expected a JSON body"})
        if length > MAX_BODY:
            return self._reply(413, {"msg": "body too large"})
        try:
            payload = json.loads(self.rfile.read(length))
            requested = str(payload["address"]).strip()
        except Exception:
            return self._reply(400, {"msg": "expected JSON {\"address\": \"0x...\"}"})

        if not is_hex_address(requested):
            return self._reply(400, {"msg": "invalid address"})
        # Accept any case, but treat a supplied mixed-case form as a checksum and
        # verify it, it is a useful typo guard.
        raw = requested[2:] if requested.lower().startswith("0x") else requested
        if raw != raw.lower() and raw != raw.upper():
            if to_checksum_address("0x" + raw.lower()) != "0x" + raw:
                return self._reply(400, {"msg": "address checksum mismatch"})
        to = to_checksum_address("0x" + raw.lower())
        # Key limits off the normalized form: otherwise "0xabc…" and "abc…" are
        # two buckets for one account.
        lowered = to.lower()
        if int(to, 16) == 0:
            return self._reply(400, {"msg": "refusing the zero address"})

        if not SENDER.claim_inflight(lowered):
            return self._reply(429, {"msg": "a claim for that address is already in flight"})
        try:
            # Reserve the windows before doing any RPC work, so a caller that is
            # already limited cannot use us to amplify load against the node.
            keys = [(client_bucket(self), PER_IP_WINDOW), (f"addr:{lowered}", PER_ADDR_WINDOW)]
            denial, token = LIMITER.check(keys)
            if denial:
                return self._reply(429, {"msg": denial})

            try:
                balance = int(rpc("eth_getBalance", [to, "latest"]), 16)
                funds = int(rpc("eth_getBalance", [ACCT.address, "latest"]), 16)
            except Exception as err:
                LIMITER.release(keys, token)
                print(f"rpc unavailable: {err}", flush=True)
                return self._reply(502, {"msg": "node unavailable, try again shortly"})
            if balance >= MAX_RECIPIENT_WEI:
                LIMITER.release(keys, token)
                return self._reply(400, {"msg": "that address already holds enough"})
            if funds < AMOUNT_WEI + MIN_RESERVE_WEI:
                LIMITER.release(keys, token)
                return self._reply(503, {"msg": "faucet is empty, ask an operator to top it up"})

            try:
                tx_hash = SENDER.send(to)
            except RpcError as err:
                # The node rejected it, so nothing was sent: give the windows back.
                LIMITER.release(keys, token)
                print(f"send rejected: {err}", flush=True)
                return self._reply(400, {"msg": f"could not send: {err}"})
            except Exception as err:
                # Unknown outcome, the transaction may be live. Fail closed and
                # keep the window, or a slow node becomes a free-ETH loop.
                print(f"send outcome unknown: {err}", flush=True)
                return self._reply(504, {"msg": "send timed out, outcome unknown; "
                                                "check your balance before retrying"})

            return self._await_receipt(tx_hash)
        finally:
            SENDER.drop_inflight(lowered)

    def _same_origin(self, origin):
        host = (self.headers.get("Host") or "").split(":")[0].lower()
        return origin.split("//")[-1].split(":")[0].lower() == host

    def _await_receipt(self, tx_hash):
        """Report what actually happened rather than assuming inclusion."""
        for _ in range(RECEIPT_POLL_SECONDS):
            time.sleep(1)
            try:
                receipt = rpc("eth_getTransactionReceipt", [tx_hash])
            except Exception:
                # An RPC error is not "not yet mined". Fall back to the tx's own
                # block number, which does not depend on receipt storage.
                receipt = None
                try:
                    tx = rpc("eth_getTransactionByHash", [tx_hash])
                    if tx and tx.get("blockNumber"):
                        return self._reply(200, {"msg": "sent", "txhash": tx_hash})
                except Exception:
                    pass
            if receipt:
                if receipt.get("status") == "0x1":
                    return self._reply(200, {"msg": "sent", "txhash": tx_hash})
                return self._reply(502, {"msg": "transaction failed on chain",
                                         "txhash": tx_hash})
        # Never mined within the window: the nonce may be stranded, so make the
        # next claim re-read it from chain.
        SENDER.invalidate_nonce()
        return self._reply(202, {"msg": "submitted, not yet mined", "txhash": tx_hash})


if __name__ == "__main__":
    print(f"faucet {ACCT.address} dispensing {AMOUNT_WEI / 10**18:g} ETH "
          f"on {BIND_ADDR}:{PORT}", flush=True)
    server = ThreadingHTTPServer((BIND_ADDR, PORT), Handler)
    server.request_queue_size = 128
    server.daemon_threads = True
    server.serve_forever()

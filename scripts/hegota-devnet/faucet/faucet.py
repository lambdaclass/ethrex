#!/usr/bin/env python3
"""Hegotá devnet faucet.

Dispenses test ETH over `POST /api/claim {"address": "0x..."}`, estimating gas so
it works for addresses that do not exist yet (EIP-8038 prices account creation
well above the historical 21000, which is what broke the off-the-shelf faucet).

Rate limiting, and why each rule exists, is documented in DESIGN.md.

Config (env):
  RPC_URL                  execution RPC                      (required)
  PRIVATE_KEY              funding key, dedicated to this      (required)
  AMOUNT_ETH               dispensed per claim                 (default 1)
  PER_IP_MINUTES           per-IP-bucket window                (default 60)
  PER_ADDR_MINUTES         per-recipient window                (default 60)
  GLOBAL_PER_HOUR          cap on total claims per hour        (default 100)
  MIN_RESERVE_ETH          refuse below this balance           (default 100)
  MAX_RECIPIENT_ETH        refuse recipients richer than this  (default 10)
  PROXY_COUNT              trusted proxy hops in front         (default 1)
  PORT                                                         (default 8080)
"""
import ipaddress
import json
import os
import threading
import time
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import request as urlrequest

from eth_account import Account  # type: ignore
from eth_utils import is_hex_address, to_checksum_address  # type: ignore

RPC_URL = os.environ["RPC_URL"]
ACCT = Account.from_key(os.environ["PRIVATE_KEY"])
AMOUNT_WEI = int(float(os.environ.get("AMOUNT_ETH", "1")) * 10**18)
PER_IP_WINDOW = int(os.environ.get("PER_IP_MINUTES", "60")) * 60
PER_ADDR_WINDOW = int(os.environ.get("PER_ADDR_MINUTES", "60")) * 60
GLOBAL_PER_HOUR = int(os.environ.get("GLOBAL_PER_HOUR", "100"))
MIN_RESERVE_WEI = int(float(os.environ.get("MIN_RESERVE_ETH", "100")) * 10**18)
MAX_RECIPIENT_WEI = int(float(os.environ.get("MAX_RECIPIENT_ETH", "10")) * 10**18)
PROXY_COUNT = int(os.environ.get("PROXY_COUNT", "1"))
PORT = int(os.environ.get("PORT", "8080"))

MAX_BODY = 1024
MAX_TRACKED = 100_000
GAS_MARGIN_NUM, GAS_MARGIN_DEN = 5, 4  # 1.25x
GAS_FALLBACK = 300_000


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    req = urlrequest.Request(RPC_URL, data=body.encode(),
                             headers={"content-type": "application/json"})
    with urlrequest.urlopen(req, timeout=15) as resp:
        out = json.loads(resp.read())
    if "error" in out:
        raise RuntimeError(out["error"].get("message", "rpc error"))
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
        stale = [k for k, exp in self.seen.items() if exp <= now]
        for k in stale:
            del self.seen[k]
        if len(self.seen) > MAX_TRACKED:
            for k, _ in sorted(self.seen.items(), key=lambda kv: kv[1])[: len(self.seen) // 4]:
                del self.seen[k]
        while self.global_hits and now - self.global_hits[0] > 3600:
            self.global_hits.popleft()

    def check(self, keys_with_windows):
        """Reserve every key at once, or reject without consuming any of them."""
        now = time.time()
        with self.lock:
            self._prune(now)
            if len(self.global_hits) >= GLOBAL_PER_HOUR:
                return "the faucet is rate limited globally, try again later"
            for key, window in keys_with_windows:
                exp = self.seen.get(key)
                if exp and exp > now:
                    return f"rate limited, try again in {int(exp - now) // 60 + 1} minute(s)"
            for key, window in keys_with_windows:
                self.seen[key] = now + window
            self.global_hits.append(now)
            return None

    def release(self, keys_with_windows):
        """Give the windows back when the send itself failed."""
        with self.lock:
            for key, _ in keys_with_windows:
                self.seen.pop(key, None)
            if self.global_hits:
                self.global_hits.pop()


class Sender:
    """Serializes sending so concurrent claims cannot collide on the nonce."""

    def __init__(self):
        self.lock = threading.Lock()
        self.nonce = None
        self.inflight: set[str] = set()

    def claim_inflight(self, addr):
        with self.lock:
            if addr in self.inflight:
                return False
            self.inflight.add(addr)
            return True

    def drop_inflight(self, addr):
        with self.lock:
            self.inflight.discard(addr)

    def _resync(self):
        self.nonce = int(rpc("eth_getTransactionCount", [ACCT.address, "pending"]), 16)

    def send(self, to):
        with self.lock:
            if self.nonce is None:
                self._resync()
            for attempt in (1, 2):
                try:
                    return self._send_once(to)
                except RuntimeError as err:
                    msg = str(err).lower()
                    retryable = "nonce" in msg or "already known" in msg
                    if attempt == 2 or not retryable:
                        raise
                    self._resync()
            raise RuntimeError("unreachable")

    def _send_once(self, to):
        chain_id = int(rpc("eth_chainId", []), 16)
        head = rpc("eth_getBlockByNumber", ["latest", False])
        base_fee = int(head.get("baseFeePerGas", "0x0"), 16)
        tip = 10**9
        try:
            est = int(rpc("eth_estimateGas", [{
                "from": ACCT.address, "to": to, "value": hex(AMOUNT_WEI)}]), 16)
            gas = est * GAS_MARGIN_NUM // GAS_MARGIN_DEN
        except RuntimeError:
            gas = GAS_FALLBACK
        tx = {
            "type": 2,
            "chainId": chain_id,
            "nonce": self.nonce,
            "to": to,
            "value": AMOUNT_WEI,
            "gas": gas,
            "maxPriorityFeePerGas": tip,
            "maxFeePerGas": base_fee * 2 + tip,
            "accessList": [],
        }
        signed = ACCT.sign_transaction(tx)
        tx_hash = rpc("eth_sendRawTransaction", ["0x" + signed.raw_transaction.hex()])
        self.nonce += 1
        return tx_hash


LIMITER = Limiter()
SENDER = Sender()


def client_bucket(handler):
    """Client IP per the trusted-proxy count, bucketed by /64 for IPv6.

    `X-Forwarded-For` is appended to by each proxy, so with N trusted hops in
    front the client is the Nth entry from the right. Anything further left is
    client-supplied and must not be trusted.
    """
    raw = handler.headers.get("X-Forwarded-For", "")
    parts = [p.strip() for p in raw.split(",") if p.strip()]
    ip_text = parts[-PROXY_COUNT] if len(parts) >= PROXY_COUNT >= 1 else handler.client_address[0]
    try:
        ip = ipaddress.ip_address(ip_text)
    except ValueError:
        return f"ip:{ip_text}"
    if ip.version == 6:
        return f"ip6:{ipaddress.ip_network(f'{ip}/64', strict=False).network_address}"
    return f"ip4:{ip}"


FORM = b"""<!doctype html><meta charset=utf-8><title>Hegota devnet faucet</title>
<style>body{font-family:system-ui;margin:4rem auto;max-width:32rem}
input{width:100%;padding:.6rem;font-family:monospace}
button{margin-top:.6rem;padding:.6rem 1rem}pre{background:#f4f4f4;padding:.8rem;white-space:pre-wrap}</style>
<h1>Hegot&aacute; devnet faucet</h1>
<p>Paste an address to receive test ETH.</p>
<input id=a placeholder="0x..." spellcheck=false><button onclick=go()>Request</button>
<pre id=o></pre><script>
async function go(){const o=document.getElementById('o');o.textContent='sending...';
try{const r=await fetch('/api/claim',{method:'POST',headers:{'content-type':'application/json'},
body:JSON.stringify({address:document.getElementById('a').value.trim()})});
o.textContent=JSON.stringify(await r.json(),null,2)}catch(e){o.textContent=String(e)}}
</script>"""


class Handler(BaseHTTPRequestHandler):
    server_version = "hegota-faucet"
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # keep the funding key and bodies out of logs
        print(f"{self.address_string()} {fmt % args}", flush=True)

    def _reply(self, code, payload):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/") in ("", "/index.html"):
            self.send_response(200)
            self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(FORM)))
            self.end_headers()
            self.wfile.write(FORM)
        elif self.path == "/healthz":
            self._reply(200, {"ok": True, "faucet": ACCT.address})
        else:
            self._reply(404, {"msg": "not found"})

    def do_POST(self):
        if self.path.rstrip("/") != "/api/claim":
            return self._reply(404, {"msg": "not found"})
        try:
            length = int(self.headers.get("content-length") or 0)
        except ValueError:
            return self._reply(400, {"msg": "bad content-length"})
        if length <= 0 or length > MAX_BODY:
            return self._reply(413, {"msg": "body too large"})
        try:
            payload = json.loads(self.rfile.read(length))
            requested = str(payload["address"]).strip()
        except Exception:
            return self._reply(400, {"msg": "expected JSON {\"address\": \"0x...\"}"})

        # Accept any case; only reject a mixed-case address that fails EIP-55,
        # where the checksum is meaningful as a typo guard.
        if not is_hex_address(requested):
            return self._reply(400, {"msg": "invalid address"})
        lowered = requested.lower()
        mixed = requested != lowered and requested != requested.upper()
        if mixed and to_checksum_address(lowered) != requested:
            return self._reply(400, {"msg": "address checksum mismatch"})
        to = to_checksum_address(lowered)
        if int(to, 16) == 0:
            return self._reply(400, {"msg": "refusing the zero address"})

        if not SENDER.claim_inflight(lowered):
            return self._reply(429, {"msg": "a claim for that address is already in flight"})
        try:
            try:
                balance = int(rpc("eth_getBalance", [to, "latest"]), 16)
                funds = int(rpc("eth_getBalance", [ACCT.address, "latest"]), 16)
            except Exception as err:
                return self._reply(502, {"msg": f"rpc unavailable: {err}"})
            if balance >= MAX_RECIPIENT_WEI:
                return self._reply(400, {"msg": "that address already holds enough"})
            if funds < AMOUNT_WEI + MIN_RESERVE_WEI:
                return self._reply(503, {"msg": "faucet is empty, ask an operator to top it up"})

            keys = [(client_bucket(self), PER_IP_WINDOW), (f"addr:{lowered}", PER_ADDR_WINDOW)]
            denial = LIMITER.check(keys)
            if denial:
                return self._reply(429, {"msg": denial})

            try:
                tx_hash = SENDER.send(to)
            except Exception as err:
                LIMITER.release(keys)  # the claim never happened; don't burn the window
                return self._reply(502, {"msg": f"send failed: {err}"})

            # Report what actually happened rather than assuming inclusion.
            for _ in range(20):
                time.sleep(1)
                try:
                    receipt = rpc("eth_getTransactionReceipt", [tx_hash])
                except Exception:
                    receipt = None
                if receipt:
                    if receipt.get("status") == "0x1":
                        return self._reply(200, {"msg": "sent", "txhash": tx_hash})
                    return self._reply(502, {"msg": "transaction failed on chain",
                                             "txhash": tx_hash})
            return self._reply(202, {"msg": "submitted, not yet mined", "txhash": tx_hash})
        finally:
            SENDER.drop_inflight(lowered)


if __name__ == "__main__":
    print(f"faucet {ACCT.address} dispensing {AMOUNT_WEI / 10**18} ETH on :{PORT}", flush=True)
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()

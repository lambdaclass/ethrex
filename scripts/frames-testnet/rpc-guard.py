#!/usr/bin/env python3
"""JSON-RPC namespace allowlist for a publicly proxied execution RPC.

`ethereum-package` starts every execution client with its full API surface —
for ethrex, `--http.api=eth,net,web3,debug,admin,txpool`
(`src/el/ethrex/ethrex_launcher.star`) — because the package is built for
private devnets where nothing outside can reach the port. Putting a reverse
proxy in front of that port publishes `debug_setHead`, `admin_addPeer`,
`admin_setLogLevel` and the tracing calls to the internet.

The namespace set cannot be narrowed from the launch command: repeated
`--http.api` flags take the union rather than replacing (pinned by
`http_api_repeated_flags_accumulate` in `cmd/ethrex/cli.rs`), so a second flag
can only add. Nor can the node simply drop `admin`: the package's own enode
discovery calls `admin_nodeInfo`, and so does
`scripts/frames-testnet/publish-artifacts.sh`.

So the split is made by reachability instead. This sits between the reverse
proxy and the node: public traffic reaches only the allowed namespaces, while
anything on the host that talks to the node's port directly — the package, the
publish script — keeps the full API.

Config (env):
  UPSTREAMS             `host=addr:port` per public hostname, comma separated
                        (required, e.g. `rpc1.example=127.0.0.1:32003`)
  ALLOWED_NAMESPACES    comma separated       (default eth,net,web3,txpool,ethrex)
  LISTEN_ADDR                                 (default 127.0.0.1)
  LISTEN_PORT                                 (default 8645)
  MAX_BODY              bytes per request     (default 1048576)
  MAX_BATCH             calls per batch       (default 100)
  UPSTREAM_TIMEOUT      seconds               (default 30)

Bind to loopback and let the reverse proxy front it. The proxy must pass the
original `Host` through, since that is what selects the upstream; an unknown
host is refused rather than sent to a default node.
"""
import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import error as urlerror
from urllib import request as urlrequest

# `ethrex` is in the default set on purpose. It holds one read-only method,
# `ethrex_simulateFrameTransaction`, and it exists as its own namespace so that
# simulating a type-0x06 envelope can be offered publicly without enabling
# `debug_` (see `map_ethrex_requests` in `crates/networking/rpc/rpc.rs`). On a
# chain no wallet can build a frame transaction for, that call is the point.
ALLOWED = tuple(
    f"{ns.strip()}_"
    for ns in os.environ.get("ALLOWED_NAMESPACES", "eth,net,web3,txpool,ethrex").split(",")
    if ns.strip()
)
LISTEN_ADDR = os.environ.get("LISTEN_ADDR", "127.0.0.1")
LISTEN_PORT = int(os.environ.get("LISTEN_PORT", "8645"))
MAX_BODY = int(os.environ.get("MAX_BODY", str(1024 * 1024)))
MAX_BATCH = int(os.environ.get("MAX_BATCH", "100"))
UPSTREAM_TIMEOUT = float(os.environ.get("UPSTREAM_TIMEOUT", "30"))


def parse_upstreams(spec):
    table = {}
    for entry in spec.split(","):
        entry = entry.strip()
        if not entry:
            continue
        host, _, target = entry.partition("=")
        if not host or not target:
            raise SystemExit(f"UPSTREAMS entry is not host=addr:port: {entry!r}")
        table[host.strip().lower()] = target.strip()
    if not table:
        raise SystemExit("UPSTREAMS is empty")
    return table


UPSTREAMS = parse_upstreams(os.environ.get("UPSTREAMS", ""))

# Hop-by-hop headers belong to a single connection and must not be relayed.
HOP_BY_HOP = frozenset(
    (
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    )
)

DENIED = {}
DENIED_LOCK = threading.Lock()


def method_allowed(name):
    return isinstance(name, str) and name.startswith(ALLOWED)


def first_denied(payload):
    """The first method in a request that is not allowed, or None.

    A batch is refused as a whole rather than split: forwarding the allowed half
    of a batch would answer a request nobody made, and mixing a real result with
    a synthesised error in one array is worse than a clear refusal.
    """
    calls = payload if isinstance(payload, list) else [payload]
    if len(calls) > MAX_BATCH:
        return f"<batch of {len(calls)} calls, limit {MAX_BATCH}>"
    for call in calls:
        if not isinstance(call, dict):
            return "<malformed call>"
        if not method_allowed(call.get("method")):
            return str(call.get("method"))
    return None


def request_id(payload):
    if isinstance(payload, dict):
        value = payload.get("id")
        if isinstance(value, (str, int)) or value is None:
            return value
    return None


class Handler(BaseHTTPRequestHandler):
    server_version = "hegota-rpc-guard"
    protocol_version = "HTTP/1.1"
    timeout = 15

    def log_message(self, fmt, *args):
        # Allowed calls are not logged: this sits in front of a public endpoint
        # and per-request logging is a disk-fill vector. Refusals are logged
        # once per method name, which is what shows an operator it is probed.
        pass

    def _upstream(self):
        host = (self.headers.get("Host") or "").split(":")[0].strip().lower()
        return UPSTREAMS.get(host), host

    def _send(self, code, body, headers=()):
        self.send_response(code)
        for key, value in headers:
            self.send_header(key, value)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_json(self, code, payload):
        self._send(
            code,
            json.dumps(payload).encode(),
            [("content-type", "application/json")],
        )

    def _refuse(self, method, rpc_id):
        with DENIED_LOCK:
            DENIED[method] = DENIED.get(method, 0) + 1
            count = DENIED[method]
        if count in (1, 10, 100) or count % 1000 == 0:
            print(f"refused {method} ({count} so far)", file=sys.stderr, flush=True)
        # -32601 is the JSON-RPC code for a method the endpoint does not serve,
        # which is exactly true here: it exists on the node, not on this route.
        self._send_json(
            200,
            {
                "jsonrpc": "2.0",
                "id": rpc_id,
                "error": {
                    "code": -32601,
                    "message": f"{method} is not available on this endpoint",
                },
            },
        )

    def _forward(self, body):
        target, host = self._upstream()
        if target is None:
            self._send_json(
                502,
                {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32603, "message": f"no upstream for host {host}"},
                },
            )
            return

        headers = {
            key: value
            for key, value in self.headers.items()
            if key.lower() not in HOP_BY_HOP and key.lower() not in ("host", "content-length")
        }
        req = urlrequest.Request(
            f"http://{target}{self.path}",
            data=body,
            headers=headers,
            method=self.command,
        )
        try:
            with urlrequest.urlopen(req, timeout=UPSTREAM_TIMEOUT) as res:
                payload = res.read()
                status = res.status
                out = [
                    (k, v)
                    for k, v in res.headers.items()
                    if k.lower() not in HOP_BY_HOP and k.lower() != "content-length"
                ]
        except urlerror.HTTPError as err:
            payload = err.read()
            status = err.code
            out = [
                (k, v)
                for k, v in err.headers.items()
                if k.lower() not in HOP_BY_HOP and k.lower() != "content-length"
            ]
        except Exception as err:
            print(f"upstream {target} failed: {err}", file=sys.stderr, flush=True)
            self._send_json(
                502,
                {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32603, "message": "upstream unavailable"},
                },
            )
            return
        # The node's own CORS headers are relayed untouched. Adding any here
        # would duplicate `Access-Control-Allow-Origin`, which browsers and
        # MetaMask hard-reject.
        self._send(status, payload, out)

    def do_POST(self):
        try:
            length = int(self.headers.get("content-length") or 0)
        except ValueError:
            length = -1
        if length < 0 or length > MAX_BODY:
            self.close_connection = True
            self._send_json(
                413,
                {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32600, "message": "request too large"},
                },
            )
            return
        body = self.rfile.read(length) if length else b""

        # Fail closed: a body this cannot read is a body it cannot police.
        try:
            payload = json.loads(body)
        except Exception:
            self._send_json(
                400,
                {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32700, "message": "parse error"},
                },
            )
            return

        denied = first_denied(payload)
        if denied is not None:
            self._refuse(denied, request_id(payload))
            return
        self._forward(body)

    def do_GET(self):
        self._forward(None)

    def do_HEAD(self):
        self._forward(None)

    def do_OPTIONS(self):
        # Preflights are answered by the node, so its CORS configuration stays
        # the single source of what browsers are told.
        self._forward(None)


def main():
    server = ThreadingHTTPServer((LISTEN_ADDR, LISTEN_PORT), Handler)
    server.daemon_threads = True
    allowed = ",".join(ns.rstrip("_") for ns in ALLOWED)
    print(
        f"rpc-guard on {LISTEN_ADDR}:{LISTEN_PORT} allowing [{allowed}] "
        f"for {len(UPSTREAMS)} host(s): {', '.join(sorted(UPSTREAMS))}",
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()

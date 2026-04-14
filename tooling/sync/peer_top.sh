#!/bin/bash
# Lightweight peer viewer (no 'requests' dependency) — uses watch + curl.
# For the full TUI, use: python3 peer_top.py [endpoint]
#
# Usage: ./peer_top.sh [endpoint] [interval]
# Example: ./peer_top.sh http://localhost:18547 1

ENDPOINT="${1:-http://localhost:18547}"
INTERVAL="${2:-1}"

watch -n "$INTERVAL" 'curl -s -X POST '"$ENDPOINT"' \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"admin_peerScores\",\"params\":[],\"id\":1}" \
  | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)[\"result\"]
    s = d[\"summary\"]
    print(\"Peers: {}  Eligible: {}  Avg Score: {}  Inflight: {}\".format(
        s[\"total_peers\"], s[\"eligible_peers\"], s[\"average_score\"], s[\"total_inflight_requests\"]))
    print()
    print(\"{:>14} {:>6} {:>5} {:>5} {:>12} {:>8} {:>30}\".format(
        \"Peer ID\", \"Score\", \"Reqs\", \"Elig\", \"Caps\", \"Dir\", \"Client\"))
    print(\"-\" * 86)
    for p in sorted(d[\"peers\"], key=lambda x: x[\"score\"], reverse=True):
        pid = p[\"peer_id\"][:6] + \"..\" + p[\"peer_id\"][-4:]
        caps = \",\".join(p[\"capabilities\"])[:12]
        client = p[\"client_version\"][:30]
        d2 = p[\"connection_direction\"][:3]
        print(\"{:>14} {:>6} {:>5} {:>5} {:>12} {:>8} {:>30}\".format(
            pid, p[\"score\"], p[\"inflight_requests\"], p[\"eligible\"], caps, d2, client))
except Exception as e:
    print(\"Error: {}\".format(e))
"'

#!/bin/bash
# Usage: ./peer_top.sh [endpoint] [interval]
# Example: ./peer_top.sh http://localhost:18547 1

ENDPOINT="${1:-http://localhost:18547}"
INTERVAL="${2:-1}"

watch -n "$INTERVAL" "curl -s -X POST $ENDPOINT \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"admin_peerScores\",\"params\":[],\"id\":1}' \
  | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)[\"result\"]
    s = d[\"summary\"]
    print(f\"Peers: {s[\"total_peers\"]}  Eligible: {s[\"eligible_peers\"]}  Avg Score: {s[\"average_score\"]}  Inflight: {s[\"total_inflight_requests\"]}\")
    print()
    print(f\"{\"Peer ID\":>14} {\"Score\":>6} {\"Reqs\":>5} {\"Elig\":>5} {\"Caps\":>12} {\"Dir\":>8} {\"Client\":>30}\")
    print(\"-\" * 86)
    for p in sorted(d[\"peers\"], key=lambda x: x[\"score\"], reverse=True):
        pid = p[\"peer_id\"][:6] + \"..\" + p[\"peer_id\"][-4:]
        caps = \",\".join(p[\"capabilities\"])
        client = p[\"client_version\"][:30]
        d2 = p[\"connection_direction\"][:3]
        print(f\"{pid:>14} {p[\"score\"]:>6} {p[\"inflight_requests\"]:>5} {str(p[\"eligible\"]):>5} {caps:>12} {d2:>8} {client:>30}\")
except Exception as e:
    print(f\"Error: {e}\")
'"

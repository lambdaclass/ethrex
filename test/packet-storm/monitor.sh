#!/usr/bin/env bash
# Monitor UDP packet rate on port 30303 across the discovery bridge.
#
# Usage:  docker compose exec monitor bash /monitor.sh [seconds] [node_ip]
#
# Defaults: 10 seconds, observe all nodes. Pass a node IP (e.g. 10.55.0.10)
# to focus on one node's sent/received breakdown.

DURATION=${1:-10}
NODE_IP=${2:-10.55.0.10}

echo "=== Capturing UDP:30303 traffic for ${DURATION}s ==="
echo "    (focused node: ${NODE_IP})"
echo ""

tcpdump -i any -nn -q udp port 30303 -w /tmp/capture.pcap 2>/dev/null &
TCPDUMP_PID=$!
sleep "$DURATION"
kill "$TCPDUMP_PID" 2>/dev/null
wait "$TCPDUMP_PID" 2>/dev/null

TOTAL=$(tcpdump -nn -r /tmp/capture.pcap 2>/dev/null | wc -l)
PPS=$((TOTAL / DURATION))

echo "──────────────────────────────────────────"
echo "  Duration:        ${DURATION}s"
echo "  Total packets:   ${TOTAL}"
echo "  Avg packets/sec: ${PPS}"
echo "──────────────────────────────────────────"
echo ""

# Break down by direction for the focused node
SENT=$(tcpdump -nn -r /tmp/capture.pcap src host "$NODE_IP" 2>/dev/null | wc -l)
RECV=$(tcpdump -nn -r /tmp/capture.pcap dst host "$NODE_IP" 2>/dev/null | wc -l)

echo "  ${NODE_IP} SENT: ${SENT}  ($((SENT / DURATION)) pps)"
echo "  ${NODE_IP} RECV: ${RECV}  ($((RECV / DURATION)) pps)"
echo "──────────────────────────────────────────"
echo ""

# Per-second breakdown
echo "=== Per-second breakdown (all traffic) ==="
tcpdump -nn -r /tmp/capture.pcap 2>/dev/null \
  | awk '{print substr($1,1,8)}' \
  | sort | uniq -c | sort -rn | head -20

echo ""
echo "=== Top senders by packet count ==="
tcpdump -nn -r /tmp/capture.pcap 2>/dev/null \
  | awk '{print $3}' | cut -d. -f1-4 \
  | sort | uniq -c | sort -rn | head -20

rm -f /tmp/capture.pcap

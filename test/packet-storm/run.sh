#!/usr/bin/env bash
#
# Launches the packet-storm reproduction environment.
#
# Prerequisites: docker, openssl, python3
#
# Usage:
#   cd test/packet-storm
#   bash run.sh [N]         # generate N nodes (default 50) and start
#   bash run.sh monitor [N] # start + run the monitor after warmup
#   bash run.sh down        # tear down
set -euo pipefail
cd "$(dirname "$0")"

if [[ "${1:-}" == "down" ]]; then
    docker compose down -v --remove-orphans
    rm -rf nodekeys .env docker-compose.override.yaml
    echo "Done."
    exit 0
fi

MODE="start"
NUM_NODES=50
if [[ "${1:-}" == "monitor" ]]; then
    MODE="monitor"
    NUM_NODES="${2:-50}"
elif [[ -n "${1:-}" ]]; then
    NUM_NODES="$1"
fi

# ── 1. Generate compose + keys ────────────────────────────────────
echo "Generating ${NUM_NODES}-node network..."
python3 generate.py "$NUM_NODES"

# ── 2. Launch ──────────────────────────────────────────────────────
echo ""
echo "Building and starting ${NUM_NODES} ethrex nodes..."
echo "(First build will take a while — subsequent runs reuse the image cache)"
echo ""

docker compose up --build -d

echo ""
echo "=== Network is up (${NUM_NODES} nodes) ==="
echo ""
echo "Monitor packet rates (30s capture):"
echo "  docker compose exec monitor bash /monitor.sh 30"
echo ""
echo "Start the flooder (sends crafted Neighbors to node1, triggers 1:16 amplification):"
echo "  bash run-flooder.sh             # builds + runs natively (needs cargo)"
echo "  # or directly if already built:"
echo "  ../../target/release/packet-storm-flooder 10.55.0.10 30303 200"
echo ""
echo "Node logs:"
echo "  docker compose logs -f node1"
echo "  docker compose logs -f flooder"
echo ""
echo "Tear down:"
echo "  bash run.sh down"

if [[ "$MODE" == "monitor" ]]; then
    echo ""
    echo "Waiting 20s for discovery warmup..."
    sleep 20
    docker compose exec monitor bash /monitor.sh 30
fi

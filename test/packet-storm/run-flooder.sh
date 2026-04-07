#!/usr/bin/env bash
#
# Builds and runs the flooder natively, connected to the Docker bridge.
#
# Usage:
#   cd test/packet-storm
#   bash run-flooder.sh [target_ip] [target_port] [pps]
#
# Defaults: 10.55.0.10:30303 at 200 pps
set -euo pipefail
cd "$(dirname "$0")"

TARGET_IP="${1:-10.55.0.10}"
TARGET_PORT="${2:-30303}"
PPS="${3:-200}"

# Add flooder to workspace temporarily
ROOT="$(cd ../.. && pwd)"
if ! grep -q 'packet-storm-flooder' "$ROOT/Cargo.toml"; then
    echo "Adding flooder to workspace (temporarily)..."
    sed -i '/^members = \[/a\  "test/packet-storm/flooder",' "$ROOT/Cargo.toml"
    CLEANUP_WORKSPACE=1
else
    CLEANUP_WORKSPACE=0
fi

cleanup() {
    if [[ "$CLEANUP_WORKSPACE" == "1" ]]; then
        echo "Removing flooder from workspace..."
        sed -i '/test\/packet-storm\/flooder/d' "$ROOT/Cargo.toml"
    fi
}
trap cleanup EXIT

echo "Building flooder..."
cargo build --release -p packet-storm-flooder --manifest-path "$ROOT/Cargo.toml"

BINARY="$ROOT/target/release/packet-storm-flooder"

echo ""
echo "Running flooder against ${TARGET_IP}:${TARGET_PORT} at ${PPS} pps"
echo "Press Ctrl+C to stop"
echo ""

"$BINARY" "$TARGET_IP" "$TARGET_PORT" "$PPS"

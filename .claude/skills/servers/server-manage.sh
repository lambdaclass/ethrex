#!/usr/bin/env bash
# server-manage.sh — Rerun or restart ethrex on a server
# Usage:
#   server-manage.sh rerun  <server> <branch>   # keep DB, switch branch, rebuild, restart
#   server-manage.sh restart <server> <branch>   # clean DB, switch branch, rebuild, restart
set -euo pipefail

ACTION="${1:-}"
SERVER="${2:-}"
BRANCH="${3:-}"

if [[ -z "$ACTION" || -z "$SERVER" || -z "$BRANCH" ]]; then
    echo "Usage: $0 <rerun|restart> <server> <branch>"
    echo "  server: srv1..srv5, srv6..srv10, office1..office5"
    echo "  rerun:   keep DB, switch branch, rebuild, restart"
    echo "  restart: clean DB, switch branch, rebuild, restart"
    exit 1
fi

if [[ "$ACTION" != "rerun" && "$ACTION" != "restart" ]]; then
    echo "Error: action must be 'rerun' or 'restart', got '$ACTION'"
    exit 1
fi

# Map server shortname to SSH host
resolve_host() {
    local srv="$1"
    case "$srv" in
        srv[1-9]|srv10)
            local num="${srv#srv}"
            echo "admin@ethrex-mainnet-${num}"
            ;;
        office[1-5])
            local num="${srv#office}"
            echo "admin@ethrex-office-${num}"
            ;;
        *)
            echo ""
            ;;
    esac
}

HOST=$(resolve_host "$SERVER")
if [[ -z "$HOST" ]]; then
    echo "Error: unknown server '$SERVER'. Use srv1..srv10 or office1..office5"
    exit 1
fi

SSH_OPTS="-o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=accept-new"

echo "=== $ACTION $SERVER ($HOST) -> branch: $BRANCH ==="

# Step 1: Stop ethrex
echo "[1/5] Stopping ethrex..."
ssh $SSH_OPTS "$HOST" 'pkill -f "target.*ethrex" 2>/dev/null || true; sleep 2' 2>/dev/null

# Step 2: Clean DB if restart
if [[ "$ACTION" == "restart" ]]; then
    echo "[2/5] Cleaning DB..."
    ssh $SSH_OPTS "$HOST" bash <<'REMOTE' 2>/dev/null
DB_DIR="$HOME/.local/share/ethrex"
if [ -d "$DB_DIR" ]; then
    echo "Removing $(du -sh "$DB_DIR" 2>/dev/null | cut -f1) DB at $DB_DIR"
    rm -rf "$DB_DIR"
    echo "DB cleaned"
else
    echo "No DB found at $DB_DIR"
fi
REMOTE
else
    echo "[2/5] Keeping DB (rerun mode)"
fi

# Step 3: Fetch and checkout branch
echo "[3/5] Switching to branch: $BRANCH..."
ssh $SSH_OPTS "$HOST" bash <<REMOTE 2>/dev/null
source ~/.cargo/env 2>/dev/null || true
cd ~/ethrex
git fetch origin "$BRANCH"
git checkout "$BRANCH" 2>&1 || git checkout -b "$BRANCH" "origin/$BRANCH" 2>&1
git reset --hard "origin/$BRANCH"
echo "HEAD: \$(git log --oneline -1)"
REMOTE

# Step 4: Build
echo "[4/5] Building (this takes ~1-2 min)..."
ssh $SSH_OPTS "$HOST" bash <<'REMOTE' 2>&1 | tail -3
source ~/.cargo/env 2>/dev/null || true
cd ~/ethrex
cargo build --release --features jemalloc 2>&1
REMOTE

# Step 5: Start
echo "[5/5] Starting ethrex..."
ssh $SSH_OPTS "$HOST" bash <<'REMOTE' 2>/dev/null
cd ~/ethrex
tmux kill-session -t ethrex 2>/dev/null || true
tmux new-session -d -s ethrex './target/release/ethrex --network mainnet --http.addr 0.0.0.0 --http.port 8545 --authrpc.port 8551 --authrpc.jwtsecret ~/secrets/jwt.hex --p2p.port 30303 --discovery.port 30303 --metrics --metrics.port 3701 --log.dir /var/log/ethrex 2>&1 | tee ~/ethrex.log'
sleep 3
if pgrep -f 'target.*ethrex' > /dev/null; then
    echo "ethrex started successfully (PID $(pgrep -f 'target.*ethrex' | head -1))"
else
    echo "FAILED to start ethrex"
    tail -10 ~/ethrex.log 2>/dev/null
    exit 1
fi
REMOTE

echo ""
echo "=== Done: $SERVER now running $BRANCH ==="

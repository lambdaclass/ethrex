#!/usr/bin/env bash
# Redeploy the EIP-8141 demo: kill all services, wipe DBs, restart everything.
#
# Usage:
#   ./scripts/redeploy.sh                    # redeploy locally
#   ./scripts/redeploy.sh --blockscout-repo /path/to/ethrex-blockscout  # with Blockscout
#
# What it does:
#   1. Kills ethrex, backend, frontend processes on ports 8545/3000/5173
#   2. Wipes the ethrex dev DB (~/Library/Application Support/ethrex/dev/ on macOS,
#      ~/.local/share/ethrex/dev/ on Linux)
#   3. If --blockscout-repo is provided, wipes and restarts Blockscout Docker containers
#   4. Starts ethrex, backend, frontend in background
#   5. Waits for all services to be healthy
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DEMO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"

BLOCKSCOUT_REPO=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --blockscout-repo) BLOCKSCOUT_REPO="$2"; shift 2;;
    *) echo "Unknown option: $1"; exit 1;;
  esac
done

echo "=== EIP-8141 Demo Redeploy ==="
echo "Demo dir: $DEMO_DIR"

# ── Step 1: Kill running services ──
echo ""
echo "[1/5] Killing running services..."
for port in 8545 3000 5173; do
  pids=$(lsof -ti:"$port" 2>/dev/null || true)
  if [ -n "$pids" ]; then
    echo "  Killing PIDs on port $port: $pids"
    echo "$pids" | xargs kill -9 2>/dev/null || true
  fi
done
sleep 1

# ── Step 2: Wipe ethrex dev DB ──
echo ""
echo "[2/5] Wiping ethrex dev database..."
if [[ "$(uname)" == "Darwin" ]]; then
  DB_PATH="$HOME/Library/Application Support/ethrex/dev"
else
  DB_PATH="$HOME/.local/share/ethrex/dev"
fi
if [ -d "$DB_PATH" ]; then
  rm -rf "$DB_PATH"
  echo "  Deleted: $DB_PATH"
else
  echo "  No DB found at: $DB_PATH (already clean)"
fi

# ── Step 3: Wipe and restart Blockscout (if repo provided) ──
if [ -n "$BLOCKSCOUT_REPO" ]; then
  echo ""
  echo "[3/5] Restarting Blockscout with clean database..."
  COMPOSE_DIR="$BLOCKSCOUT_REPO/docker-compose"
  if [ ! -f "$COMPOSE_DIR/docker-compose.yml" ]; then
    echo "  ERROR: docker-compose.yml not found at $COMPOSE_DIR"
    exit 1
  fi
  cd "$COMPOSE_DIR"
  docker compose down 2>/dev/null || true
  # The DB bind mount is relative to the services/ subdir (see services/db.yml)
  rm -rf services/blockscout-db-data services/stats-db-data services/redis-data logs dets 2>/dev/null || true
  echo "  Deleted Blockscout data directories"
  docker compose up -d 2>&1 | tail -3
  echo "  Blockscout started (indexing will begin in ~30s)"
  cd "$DEMO_DIR"
else
  echo ""
  echo "[3/5] Skipping Blockscout (use --blockscout-repo to include)"
fi

# ── Step 4: Start services ──
echo ""
echo "[4/5] Starting services..."

# ethrex node
echo "  Starting ethrex node..."
cd "$DEMO_DIR"
cargo run --manifest-path "$REPO_ROOT/Cargo.toml" --bin ethrex --features dev -- \
  --network "$DEMO_DIR/genesis.json" --http.port 8545 --dev \
  > /tmp/ethrex-demo.log 2>&1 &
ETHREX_PID=$!
echo "  ethrex PID: $ETHREX_PID (log: /tmp/ethrex-demo.log)"

# Wait for ethrex to be ready
echo "  Waiting for ethrex..."
for i in $(seq 1 60); do
  if curl -s -o /dev/null -w '' --max-time 2 -X POST \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
    http://localhost:8545 2>/dev/null; then
    echo "  ethrex ready after ${i}s"
    break
  fi
  if [ "$i" -eq 60 ]; then
    echo "  ERROR: ethrex failed to start. Check /tmp/ethrex-demo.log"
    exit 1
  fi
  sleep 1
done

# Backend
echo "  Starting backend..."
cd "$DEMO_DIR/backend"
npx tsx src/index.ts > /tmp/demo-backend.log 2>&1 &
BACKEND_PID=$!
echo "  Backend PID: $BACKEND_PID (log: /tmp/demo-backend.log)"

# Frontend
echo "  Starting frontend..."
cd "$DEMO_DIR/frontend"
npx vite --host > /tmp/demo-frontend.log 2>&1 &
FRONTEND_PID=$!
echo "  Frontend PID: $FRONTEND_PID (log: /tmp/demo-frontend.log)"

# ── Step 5: Verify all services ──
echo ""
echo "[5/5] Verifying services..."
sleep 3

ETHREX_OK=false
BACKEND_OK=false
FRONTEND_OK=false

code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:8545 2>/dev/null || echo "000")
[ "$code" = "200" ] && ETHREX_OK=true

code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 http://localhost:3000/health 2>/dev/null || echo "000")
[ "$code" = "200" ] && BACKEND_OK=true

code=$(curl -sk -o /dev/null -w '%{http_code}' --max-time 3 https://localhost:5173 2>/dev/null || echo "000")
[ "$code" = "200" ] && FRONTEND_OK=true

echo ""
echo "=== Status ==="
echo "  ethrex  (8545):  $($ETHREX_OK && echo 'OK' || echo 'FAILED')"
echo "  backend (3000):  $($BACKEND_OK && echo 'OK' || echo 'FAILED')"
echo "  frontend(5173):  $($FRONTEND_OK && echo 'OK' || echo 'FAILED')"

if [ -n "$BLOCKSCOUT_REPO" ]; then
  bs_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 http://localhost:8082 2>/dev/null || echo "000")
  echo "  blockscout(8082): $([ "$bs_code" = "200" ] && echo 'OK' || echo 'STARTING...')"
fi

if $ETHREX_OK && $BACKEND_OK && $FRONTEND_OK; then
  echo ""
  echo "Demo is running at https://localhost:5173"
  echo "Logs: /tmp/ethrex-demo.log, /tmp/demo-backend.log, /tmp/demo-frontend.log"
else
  echo ""
  echo "Some services failed to start. Check logs above."
  exit 1
fi

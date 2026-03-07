#!/bin/bash
# Desktop App 로컬 개발 실행 스크립트
# 사용법: cd crates/desktop-app && ./dev.sh
#
# 실행되는 서비스:
#   1. Local Server (port 5002) - Docker deployment engine
#   2. Tauri Dev (Vite + Rust) - Desktop app
#
# 종료: Ctrl+C

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "========================================"
echo "  Tokamak Desktop App - Development"
echo "========================================"
echo ""

# Check prerequisites
if ! command -v node &> /dev/null; then
  echo "❌ Node.js not found"
  exit 1
fi

if ! command -v cargo &> /dev/null; then
  echo "❌ Rust/Cargo not found"
  exit 1
fi

echo "Node: $(node -v)"
echo "Cargo: $(cargo --version)"
echo ""

# Check deps
if [ ! -d "$SCRIPT_DIR/local-server/node_modules" ]; then
  echo "📦 Installing local-server dependencies..."
  cd "$SCRIPT_DIR/local-server" && npm install
fi

if [ ! -d "$SCRIPT_DIR/ui/node_modules" ]; then
  echo "📦 Installing UI dependencies..."
  cd "$SCRIPT_DIR/ui" && pnpm install
fi

# Cleanup
cleanup() {
  echo ""
  echo "Stopping services..."
  kill $LOCAL_SERVER_PID 2>/dev/null
  wait $LOCAL_SERVER_PID 2>/dev/null
  echo "Done."
}
trap cleanup EXIT INT TERM

# Start Local Server
echo "Starting Local Server (port 5002)..."
cd "$SCRIPT_DIR/local-server"
node server.js &
LOCAL_SERVER_PID=$!

sleep 1
if ! kill -0 $LOCAL_SERVER_PID 2>/dev/null; then
  echo "❌ Local server failed to start"
  exit 1
fi

# Start Tauri Dev
echo "Starting Tauri Dev..."
cd "$SCRIPT_DIR/ui"
npx tauri dev

# tauri dev will block, cleanup happens on exit

#!/bin/bash
# Platform 로컬 개발 실행 스크립트
# 사용법: cd platform && ./dev.sh
#
# 실행되는 서비스:
#   1. Platform API Server (port 5001)
#   2. Platform Client (port 3000)
#
# 종료: Ctrl+C

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "========================================"
echo "  Tokamak Platform - Local Development"
echo "========================================"
echo ""

# Check node
if ! command -v node &> /dev/null; then
  echo "❌ Node.js not found. Please install Node.js 18+"
  exit 1
fi

echo "Node: $(node -v)"
echo ""

# Check deps
if [ ! -d "$SCRIPT_DIR/server/node_modules" ]; then
  echo "📦 Installing server dependencies..."
  cd "$SCRIPT_DIR/server" && npm install
fi

if [ ! -d "$SCRIPT_DIR/client/node_modules" ]; then
  echo "📦 Installing client dependencies..."
  cd "$SCRIPT_DIR/client" && npm install
fi

# Cleanup on exit
cleanup() {
  echo ""
  echo "Stopping services..."
  kill $SERVER_PID $CLIENT_PID 2>/dev/null
  wait $SERVER_PID $CLIENT_PID 2>/dev/null
  echo "Done."
}
trap cleanup EXIT INT TERM

# Start Platform API Server
echo "Starting Platform API Server (port 5001)..."
cd "$SCRIPT_DIR/server"
node server.js &
SERVER_PID=$!

# Wait for server to be ready
sleep 2
if ! kill -0 $SERVER_PID 2>/dev/null; then
  echo "❌ Server failed to start"
  exit 1
fi

# Start Platform Client (Next.js dev)
echo "Starting Platform Client (port 3000)..."
cd "$SCRIPT_DIR/client"
npx next dev --port 3000 &
CLIENT_PID=$!

echo ""
echo "========================================"
echo "  Services Running:"
echo "  API Server: http://localhost:5001"
echo "  Client:     http://localhost:3000"
echo "========================================"
echo ""
echo "Press Ctrl+C to stop all services"
echo ""

# Wait for any child to exit
wait

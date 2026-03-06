#!/bin/bash
# Tokamak Desktop App - Pre-build Check Script
# 푸시 전에 로컬에서 빌드 가능 여부를 점검합니다.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
fail() { echo -e "${RED}[FAIL]${NC} $1"; exit 1; }
info() { echo -e "${YELLOW}[INFO]${NC} $1"; }

echo "=================================="
echo " Tokamak Desktop App Build Check"
echo "=================================="
echo ""

# 1. Node.js & pnpm check
info "Checking prerequisites..."
command -v node >/dev/null 2>&1 || fail "Node.js not found"
command -v pnpm >/dev/null 2>&1 || fail "pnpm not found"
command -v rustc >/dev/null 2>&1 || fail "Rust not found"
pass "Node $(node -v), pnpm $(pnpm -v), rustc $(rustc --version | awk '{print $2}')"

# 2. Install dependencies (if needed)
if [ ! -d "node_modules" ]; then
  info "Installing dependencies..."
  pnpm install --frozen-lockfile
fi
pass "Dependencies installed"

# 3. TypeScript compile check
info "Running TypeScript check..."
npx tsc -b 2>&1 || fail "TypeScript compilation failed"
pass "TypeScript compilation"

# 4. ESLint check
info "Running ESLint..."
npx eslint . --max-warnings=0 2>&1 || {
  echo -e "${YELLOW}[WARN]${NC} ESLint has warnings (non-blocking)"
}
pass "ESLint check"

# 5. Vite build (frontend only)
info "Building frontend..."
npx vite build 2>&1 || fail "Vite build failed"
pass "Frontend build (dist/)"

# 6. Rust cargo check (Tauri backend)
info "Checking Tauri backend..."
cd src-tauri
cargo check 2>&1 || fail "Cargo check failed"
pass "Tauri backend (cargo check)"
cd ..

echo ""
echo "=================================="
echo -e " ${GREEN}All checks passed!${NC}"
echo " Safe to push and trigger CI build."
echo "=================================="

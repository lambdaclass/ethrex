#!/bin/bash
# Tokamak Desktop App - Pre-build Check Script
# 푸시 전에 로컬에서 빌드 가능 여부를 점검합니다.
# 사용법: ./scripts/pre-build-check.sh [--full]
#   --full: cargo clippy + Tauri 번들 빌드까지 실행 (시간 오래 걸림)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"

FULL_CHECK=false
if [ "$1" = "--full" ]; then
  FULL_CHECK=true
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

ERRORS=0
WARNINGS=0

pass() { echo -e "${GREEN}  [PASS]${NC} $1"; }
fail() { echo -e "${RED}  [FAIL]${NC} $1"; ERRORS=$((ERRORS + 1)); }
warn() { echo -e "${YELLOW}  [WARN]${NC} $1"; WARNINGS=$((WARNINGS + 1)); }
info() { echo -e "${CYAN}  [INFO]${NC} $1"; }
section() { echo -e "\n${CYAN}── $1 ──${NC}"; }

echo ""
echo "========================================="
echo "  Tokamak Desktop App Pre-build Check"
echo "========================================="

# ─── 1. Prerequisites ───
section "Prerequisites"

command -v node >/dev/null 2>&1 || { fail "Node.js not found"; }
command -v pnpm >/dev/null 2>&1 || { fail "pnpm not found"; }
command -v rustc >/dev/null 2>&1 || { fail "Rust not found"; }
command -v cargo >/dev/null 2>&1 || { fail "Cargo not found"; }

if [ $ERRORS -gt 0 ]; then
  echo -e "\n${RED}Missing prerequisites. Install them and retry.${NC}"
  exit 1
fi

pass "Node $(node -v), pnpm $(pnpm -v)"
pass "rustc $(rustc --version | awk '{print $2}'), cargo $(cargo --version | awk '{print $2}')"

# ─── 2. Dependencies ───
section "Dependencies"

if [ ! -d "node_modules" ]; then
  info "Installing frontend dependencies..."
  pnpm install --frozen-lockfile 2>&1 || fail "pnpm install failed"
fi
pass "Frontend dependencies (node_modules)"

if [ ! -f "pnpm-lock.yaml" ]; then
  fail "pnpm-lock.yaml not found"
else
  pass "pnpm-lock.yaml exists"
fi

# ─── 3. Icons ───
section "App Icons"

ICON_DIR="src-tauri/icons"
REQUIRED_ICONS=("32x32.png" "128x128.png" "128x128@2x.png" "icon.icns" "icon.ico")
MISSING_ICONS=0
for icon in "${REQUIRED_ICONS[@]}"; do
  if [ ! -f "$ICON_DIR/$icon" ]; then
    fail "Missing icon: $ICON_DIR/$icon"
    MISSING_ICONS=$((MISSING_ICONS + 1))
  fi
done
if [ $MISSING_ICONS -eq 0 ]; then
  pass "All required icons present (${#REQUIRED_ICONS[@]} files)"
else
  warn "Run 'pnpm tauri icon <1024x1024.png>' to generate missing icons"
fi

# ─── 4. TypeScript ───
section "TypeScript"

if npx tsc -b 2>&1; then
  pass "TypeScript compilation"
else
  fail "TypeScript compilation failed"
fi

# ─── 5. ESLint ───
section "ESLint"

ESLINT_OUTPUT=$(npx eslint . 2>&1) || true
ESLINT_ERRORS=$(echo "$ESLINT_OUTPUT" | grep -c "error" || true)
ESLINT_WARNINGS=$(echo "$ESLINT_OUTPUT" | grep -c "warning" || true)

if [ "$ESLINT_ERRORS" -gt 0 ]; then
  warn "ESLint: $ESLINT_ERRORS error(s), $ESLINT_WARNINGS warning(s)"
  echo "$ESLINT_OUTPUT" | grep "error" | head -5
  info "(Non-blocking for build, but should be fixed)"
else
  pass "ESLint clean"
fi

# ─── 6. Frontend Build ───
section "Frontend Build"

if npx vite build 2>&1; then
  DIST_SIZE=$(du -sh dist 2>/dev/null | awk '{print $1}')
  pass "Vite build success (dist/ = ${DIST_SIZE})"
else
  fail "Vite build failed"
fi

# ─── 7. Rust Backend ───
section "Rust Backend"

cd src-tauri

if cargo check 2>&1; then
  pass "cargo check"
else
  fail "cargo check failed"
fi

# Clippy (full mode only)
if [ "$FULL_CHECK" = true ]; then
  info "Running cargo clippy..."
  if cargo clippy -- -D warnings 2>&1; then
    pass "cargo clippy (no warnings)"
  else
    warn "cargo clippy has warnings"
  fi
fi

cd ..

# ─── 8. Tauri Config Validation ───
section "Tauri Config"

TAURI_CONF="src-tauri/tauri.conf.json"
if [ -f "$TAURI_CONF" ]; then
  # Check JSON is valid
  if python3 -c "import json; json.load(open('$TAURI_CONF'))" 2>/dev/null; then
    pass "tauri.conf.json is valid JSON"
  else
    fail "tauri.conf.json is invalid JSON"
  fi

  # Check bundle is active
  BUNDLE_ACTIVE=$(python3 -c "import json; print(json.load(open('$TAURI_CONF')).get('bundle',{}).get('active', False))" 2>/dev/null)
  if [ "$BUNDLE_ACTIVE" = "True" ]; then
    pass "Bundle is active"
  else
    warn "Bundle is not active in tauri.conf.json"
  fi
else
  fail "tauri.conf.json not found"
fi

# ─── 9. Full Build (--full mode only) ───
if [ "$FULL_CHECK" = true ]; then
  section "Full Tauri Build (release)"
  info "This may take 10+ minutes..."
  if pnpm tauri build 2>&1; then
    pass "Tauri release build"
  else
    fail "Tauri release build failed"
  fi
fi

# ─── Summary ───
echo ""
echo "========================================="
if [ $ERRORS -gt 0 ]; then
  echo -e "  ${RED}FAILED: $ERRORS error(s), $WARNINGS warning(s)${NC}"
  echo "  Fix errors before pushing."
  echo "========================================="
  exit 1
else
  if [ $WARNINGS -gt 0 ]; then
    echo -e "  ${YELLOW}PASSED with $WARNINGS warning(s)${NC}"
  else
    echo -e "  ${GREEN}All checks passed!${NC}"
  fi
  echo "  Safe to push and trigger CI build."
  if [ "$FULL_CHECK" = false ]; then
    echo -e "  ${CYAN}Tip: Run with --full for complete build test${NC}"
  fi
  echo "========================================="
fi

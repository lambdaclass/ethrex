#!/usr/bin/env bash
# smart_search.sh - Enhanced Lean theorem search with API integration
# Usage: ./smart_search.sh <query> [--source=leansearch|loogle|mathlib|all]
set -euo pipefail

QUERY="${1:-}"
SOURCE="mathlib"
MATHLIB_PATH="${MATHLIB_PATH:-.lake/packages/mathlib}"

for arg in "$@"; do [[ "$arg" == --source=* ]] && SOURCE="${arg#--source=}"; done

USE_RG=false; command -v rg &>/dev/null && USE_RG=true

[[ -z "$QUERY" ]] && { echo "Usage: $0 <query> [--source=leansearch|loogle|mathlib|all]" >&2; exit 1; }

search_leansearch() {
  echo "--- LeanSearch (semantic) ---"
  ENCODED=$(printf '%s' "$QUERY" | jq -sRr @uri)
  RESPONSE=$(curl -s "https://leansearch.net/api/search?query=$ENCODED&num_results=5" || echo "ERROR")
  [[ "$RESPONSE" == "ERROR" || -z "$RESPONSE" ]] && { echo "LeanSearch request failed (rate limit?)"; return 1; }
  echo "$RESPONSE" | jq -r '.results // [] | .[] | "[\(.score | tonumber | round)] \(.name)\n  \(.type)\n  \(.module)\n"' 2>/dev/null || echo "$RESPONSE"
}

search_loogle() {
  echo "--- Loogle (type-based) ---"
  ENCODED=$(printf '%s' "$QUERY" | jq -sRr @uri)
  RESPONSE=$(curl -s "https://loogle.lean-lang.org/json?q=$ENCODED" || echo "ERROR")
  [[ "$RESPONSE" == "ERROR" || -z "$RESPONSE" ]] && { echo "Loogle request failed (rate limit?)"; return 1; }
  echo "$RESPONSE" | jq -r '.hits // [] | .[0:8][] | "\(.name)\n  Type: \(.type)\n  Module: \(.module)\n"' 2>/dev/null || echo "$RESPONSE"
}

search_mathlib() {
  echo "--- Mathlib (local) ---"
  [[ ! -d "$MATHLIB_PATH" ]] && { echo "mathlib not found at $MATHLIB_PATH" >&2; return 1; }
  if [[ "$USE_RG" == true ]]; then
    rg -t lean "^(theorem|lemma|def).*$QUERY" "$MATHLIB_PATH" -n --heading --color=always | head -30
  else
    find "$MATHLIB_PATH" -name "*.lean" -type f -exec grep -l "^\(theorem\|lemma\|def\).*$QUERY" {} \; | head -10
  fi
}

case "$SOURCE" in
  leansearch) search_leansearch ;;
  loogle) search_loogle ;;
  mathlib) search_mathlib ;;
  all) search_mathlib; echo; search_leansearch || true; echo; search_loogle || true ;;
  *) echo "Invalid source: $SOURCE" >&2; exit 1 ;;
esac

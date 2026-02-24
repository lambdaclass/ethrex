#!/usr/bin/env bash
# check_axioms_inline.sh - Check axioms in Lean 4 files using inline #print axioms
# Usage: ./check_axioms_inline.sh <file-or-pattern> [--verbose]
# Standard axioms (propext, quot.sound, Classical.choice) are filtered out.
set -euo pipefail

VERBOSE=""
FILES=()
MARKER="-- AUTO_AXIOM_CHECK_MARKER_DO_NOT_COMMIT"
STANDARD_AXIOMS="propext|quot.sound|Classical.choice|Quot.sound"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'

for arg in "$@"; do
  if [[ "$arg" == "--verbose" ]]; then VERBOSE="--verbose"
  elif [[ "$arg" == *"*"* ]]; then
    expanded=($arg)
    for file in "${expanded[@]}"; do [[ -f "$file" ]] && FILES+=("$file"); done
  elif [[ -f "$arg" ]]; then FILES+=("$arg")
  else echo -e "${RED}Error: $arg is not a file${NC}" >&2; exit 1; fi
done

LEAN_FILES=()
for file in "${FILES[@]}"; do [[ "$file" =~ \.lean$ ]] && LEAN_FILES+=("$file"); done

[[ ${#LEAN_FILES[@]} -eq 0 ]] && { echo -e "${RED}No Lean files found${NC}" >&2; exit 1; }
echo -e "${BLUE}Checking axioms in ${#LEAN_FILES[@]} file(s)${NC}\n"

TOTAL_FILES=0; TOTAL_DECLS=0; FILES_WITH_CUSTOM=0; CUSTOM_COUNT=0

check_file() {
  local FILE="$1"
  echo -e "${BLUE}File: ${YELLOW}$FILE${NC}"

  local NAMESPACE=""
  grep -q "^namespace " "$FILE" && NAMESPACE=$(grep "^namespace " "$FILE" | head -1 | sed 's/namespace //')

  local DECLARATIONS=()
  while IFS= read -r line; do
    decl=$(echo "$line" | sed -E 's/^(theorem|lemma|def) +([^ :(]+).*/\2/')
    [[ -n "$decl" ]] && { [[ -n "$NAMESPACE" ]] && DECLARATIONS+=("$NAMESPACE.$decl") || DECLARATIONS+=("$decl"); }
  done < <(grep -E '^(theorem|lemma|def) ' "$FILE" || true)

  [[ ${#DECLARATIONS[@]} -eq 0 ]] && { echo -e "  ${YELLOW}No declarations found${NC}\n"; return 0; }
  echo -e "  ${GREEN}Found ${#DECLARATIONS[@]} declarations${NC}"

  local BACKUP="${FILE}.axiom_check_backup"
  cp "$FILE" "$BACKUP"
  cleanup_file() { [[ -f "$BACKUP" ]] && mv "$BACKUP" "$FILE"; }

  echo "" >> "$FILE"; echo "$MARKER" >> "$FILE"
  for decl in "${DECLARATIONS[@]}"; do echo "#print axioms $decl" >> "$FILE"; done

  local HAS_CUSTOM=false
  if OUTPUT=$(lake env lean "$FILE" 2>&1); then
    local CURRENT_DECL=""
    while IFS= read -r line; do
      if [[ "$line" =~ ^([a-zA-Z0-9_.]+)[[:space:]]+depends[[:space:]]+on[[:space:]]+axioms: ]]; then
        CURRENT_DECL="${BASH_REMATCH[1]}"
      elif [[ "$line" =~ ^[[:space:]]*([a-zA-Z0-9_.]+)[[:space:]]*$ ]]; then
        axiom="${BASH_REMATCH[1]}"
        if [[ -n "$axiom" && ! "$axiom" =~ ^[[:space:]]*$ ]]; then
          if [[ ! "$axiom" =~ $STANDARD_AXIOMS ]]; then
            echo -e "  ${RED}! $CURRENT_DECL uses non-standard axiom: $axiom${NC}"
            HAS_CUSTOM=true; ((CUSTOM_COUNT++))
          elif [[ "$VERBOSE" == "--verbose" ]]; then
            echo -e "    ${GREEN}ok${NC} $axiom"
          fi
        fi
      fi
    done <<< "$OUTPUT"
    [[ "$HAS_CUSTOM" == true ]] && ((FILES_WITH_CUSTOM++)) || echo -e "  ${GREEN}All standard axioms${NC}"
    ((TOTAL_DECLS+=${#DECLARATIONS[@]})); ((TOTAL_FILES++))
    cleanup_file; echo; return 0
  else
    echo -e "  ${RED}Error running Lean${NC}" >&2
    cleanup_file; echo; return 1
  fi
}

FAILED=()
for file in "${LEAN_FILES[@]}"; do check_file "$file" || FAILED+=("$file"); done

echo -e "${BLUE}Summary: $TOTAL_FILES files, $TOTAL_DECLS declarations${NC}"
[[ $FILES_WITH_CUSTOM -eq 0 ]] && echo -e "${GREEN}All files use standard axioms only${NC}" || echo -e "${RED}$FILES_WITH_CUSTOM file(s) with non-standard axioms ($CUSTOM_COUNT usages)${NC}"
[[ ${#FAILED[@]} -gt 0 ]] && { echo -e "${RED}${#FAILED[@]} file(s) with errors${NC}"; exit 1; }
[[ $FILES_WITH_CUSTOM -gt 0 ]] && exit 1
exit 0

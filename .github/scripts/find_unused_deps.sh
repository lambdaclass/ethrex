#!/bin/bash

# Extract dependencies using section pattern matching
WORKSPACE_DEPS=$(awk '
  /\[workspace.dependencies\]/ {section=1; next}
  /^\[[^=]+\]/ {section=0}
  section && /=/ {
    sub(/=.*/, "");  # Remove everything after =
    gsub(/^[[:space:]]+|[[:space:]]+$/, "");  # Trim whitespace
    print
  }
' Cargo.toml)

echo "Checking for unused workspace dependencies..."

# Counter for unused dependencies
UNUSED_COUNT=0

EXCEPTIONS=(
  "ef_tests-blockchain"
  "ef_tests-state"
  "ef_tests-state_v2"
)

# For each dependency, check if it's referenced in any crate
for DEP in $WORKSPACE_DEPS; do
  if [[ " ${EXCEPTIONS[@]} " =~ " ${DEP} " ]]; then
    echo "Skipping check for whitelisted dependency: $DEP"
    continue
  fi
  echo "Checking dependency: $DEP"
  
  # Check for both workspace reference patterns:
  # 1. Dot notation: dependency.workspace = true
  # 2. Inline table: dependency = { workspace = true, ... }
  USED=$(find . -path "*/Cargo.toml" -not -path "./Cargo.toml" -exec grep -l -E "(${DEP}\.workspace\s*=\s*true|${DEP}\s*=\s*\{[^}]*workspace\s*=\s*true)" {} \;)
  
  # If no uses found, report it as unused
  if [ -z "$USED" ]; then
    # Get line number and full definition
    LINE_NUM=$(grep -n "^${DEP}\s*=" Cargo.toml | head -1 | cut -d':' -f1)
    if [ -z "$LINE_NUM" ]; then
      # Try with spaces
      LINE_NUM=$(grep -n "^\s*${DEP}\s*=" Cargo.toml | head -1 | cut -d':' -f1)
    fi
    
    # Get the definition
    DEP_DEF=$(sed -n "${LINE_NUM}p" Cargo.toml)
    
    echo "Unused workspace dependency: $DEP"
    echo "  Location: ./Cargo.toml:$LINE_NUM"
    echo "  Definition: $DEP_DEF"
    echo "----------------------------------------"
    
    # Increment the counter
    UNUSED_COUNT=$((UNUSED_COUNT + 1))
  fi
done

# Report results and exit with appropriate status
if [ $UNUSED_COUNT -gt 0 ]; then
  echo "ERROR: Found $UNUSED_COUNT unused workspace dependencies!"
  echo "Please remove these dependencies or reference them from at least one crate."
  exit 1
else
  echo "SUCCESS: All workspace dependencies are used."
  exit 0
fi

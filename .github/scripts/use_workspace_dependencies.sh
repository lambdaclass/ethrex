#!/bin/bash

EXCLUDED_PATHS=(
  "./crates/l2/prover/"
  "./hive/"
  "./crates/l2/tee/"
  "./target/"
)

should_exclude() {
  local file_path="$1"
  for excluded in "${EXCLUDED_PATHS[@]}"; do
    if [[ "$file_path" == $excluded* ]]; then
      return 0
    fi
  done
  return 1
}

# Get all workspace dependencies as a searchable list
get_workspace_deps() {
  awk '
    /\[workspace.dependencies\]/ {section=1; next}
    /^\[[^=]+\]/ {section=0}
    section && /=/ {
      sub(/=.*/, "");  # Remove everything after =
      gsub(/^[[:space:]]+|[[:space:]]+$/, "");  # Trim whitespace
      print
    }
  ' Cargo.toml
}

# Get all workspace dependencies
WORKSPACE_DEPS=$(get_workspace_deps)

# Find all Cargo.toml files except the root one
CARGO_FILES=$(find . -name "Cargo.toml" -not -path "./Cargo.toml")

# Counter for violations
VIOLATIONS=0

echo "Checking Cargo.toml files for dependencies that should use workspace inheritance..."
echo "Excluded paths: ${EXCLUDED_PATHS[*]}"

for FILE in $CARGO_FILES; do
  # Skip excluded paths
  if should_exclude "$FILE"; then
    echo "Skipping excluded path: $FILE"
    continue
  fi
  
  echo "Checking $FILE..."
  
  # Extract all dependencies from this Cargo.toml
  DEPS=$(awk '
    /^\[dependencies\]/ {section=1; next}
    /^\[/ {section=0}
    section && /=/ {
      sub(/=.*/, "");  # Remove everything after =
      gsub(/^[[:space:]]+|[[:space:]]+$/, "");  # Trim whitespace
      print
    }
  ' "$FILE")
  
  # Add dev-dependencies
  DEV_DEPS=$(awk '
    /^\[dev-dependencies\]/ {section=1; next}
    /^\[/ {section=0}
    section && /=/ {
      sub(/=.*/, "");  # Remove everything after =
      gsub(/^[[:space:]]+|[[:space:]]+$/, "");  # Trim whitespace
      print
    }
  ' "$FILE")
  
  # Add build-dependencies
  BUILD_DEPS=$(awk '
    /^\[build-dependencies\]/ {section=1; next}
    /^\[/ {section=0}
    section && /=/ {
      sub(/=.*/, "");  # Remove everything after =
      gsub(/^[[:space:]]+|[[:space:]]+$/, "");  # Trim whitespace
      print
    }
  ' "$FILE")
  
  # Combine all dependencies
  ALL_DEPS=$(echo -e "$DEPS\n$DEV_DEPS\n$BUILD_DEPS" | sort | uniq)
  
  # Check each dependency in this file
  for DEP in $ALL_DEPS; do
    # Skip empty lines
    [ -z "$DEP" ] && continue
    
    # Check if this dependency exists in workspace
    if echo "$WORKSPACE_DEPS" | grep -q "^$DEP$"; then
      # Check if the dependency is using workspace inheritance
      # We need to handle both forms:
      # 1. dep.workspace = true
      # 2. dep = { workspace = true, ... }
      
      # Check if the dependency is defined in the file
      if grep -q "^[[:space:]]*$DEP[[:space:]]*=" "$FILE"; then
        # Now check if it uses workspace inheritance in either form
        if grep -q "^[[:space:]]*$DEP[[:space:]]*\.[[:space:]]*workspace[[:space:]]*=[[:space:]]*true" "$FILE" || 
           grep -q "^[[:space:]]*$DEP[[:space:]]*=[[:space:]]*{[^}]*workspace[[:space:]]*=[[:space:]]*true" "$FILE"; then
          # Using workspace correctly
          continue
        else
          # Not using workspace
          echo "ERROR: $FILE uses '$DEP' but doesn't use workspace inheritance"
          echo "  Fix by using:"
          echo "    $DEP.workspace = true"
          VIOLATIONS=$((VIOLATIONS + 1))
        fi
      fi
    fi
  done
done

if [ $VIOLATIONS -gt 0 ]; then
  echo "Found $VIOLATIONS violations. Please fix the dependencies to use workspace inheritance."
  exit 1
else
  echo "All dependencies correctly use workspace inheritance."
  exit 0
fi

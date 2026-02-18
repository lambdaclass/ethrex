#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# run_samply.sh — Profile snap sync replay with samply
# ============================================================================

TOOL="samply"

# ---------------------------------------------------------------------------
# Configuration (override via environment)
# ---------------------------------------------------------------------------
: "${DATASET:?DATASET must be set to the captured dataset directory}"
: "${BACKEND:=rocksdb}"
: "${DB_DIR:=/tmp/snap-profile-db}"
: "${KEEP_DB:=0}"
: "${SNAP_CARGO_PROFILE:=release-with-debug}"
: "${FEATURES:=rocksdb}"
: "${TIMESTAMP:=$(date -u +%Y%m%dT%H%M%SZ)}"
: "${OUT_ROOT:=./artifacts/snapsync-profile}"

# ---------------------------------------------------------------------------
# Preflight checks
# ---------------------------------------------------------------------------
if [[ ! -f "${DATASET}/manifest.json" ]]; then
    echo "ERROR: ${DATASET}/manifest.json not found" >&2
    exit 1
fi

if ! command -v samply &>/dev/null; then
    echo "ERROR: samply not found in PATH (install: cargo install samply)" >&2
    exit 1
fi

# Check that samply supports --save-only
if ! samply record --help 2>&1 | grep -q -- '--save-only'; then
    echo "WARNING: samply does not support --save-only; profile will open in browser instead of saving to file" >&2
    SAMPLY_SAVE_ONLY=0
else
    SAMPLY_SAVE_ONLY=1
fi

OUT_DIR="${OUT_ROOT}/${TIMESTAMP}/${TOOL}"
mkdir -p "${OUT_DIR}"

cp "${DATASET}/manifest.json" "${OUT_DIR}/manifest.json"

# Compute manifest sha256 (portable)
if command -v sha256sum &>/dev/null; then
    MANIFEST_SHA256=$(sha256sum "${DATASET}/manifest.json" | awk '{print $1}')
else
    MANIFEST_SHA256=$(shasum -a 256 "${DATASET}/manifest.json" | awk '{print $1}')
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "==> Building snap_profile_replay (profile=${SNAP_CARGO_PROFILE}, features=${FEATURES})"

BUILD_RUSTFLAGS="-C force-frame-pointers=yes ${RUSTFLAGS:-}"
RUSTFLAGS="${BUILD_RUSTFLAGS}" cargo build -p snapsync_profile \
    --profile "${SNAP_CARGO_PROFILE}" \
    --bin snap_profile_replay \
    --features "${FEATURES}" \
    --manifest-path tooling/Cargo.toml

BINARY="./tooling/target/${SNAP_CARGO_PROFILE}/snap_profile_replay"
if [[ ! -x "${BINARY}" ]]; then
    echo "ERROR: binary not found at ${BINARY}" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Assemble replay command
# ---------------------------------------------------------------------------
RUN_DB_DIR="${DB_DIR}/${TIMESTAMP}"
mkdir -p "${RUN_DB_DIR}"

REPLAY_CMD=("${BINARY}" "${DATASET}" --backend "${BACKEND}" --db-dir "${RUN_DB_DIR}")
if [[ "${KEEP_DB}" == "1" ]]; then
    REPLAY_CMD+=(--keep-db)
fi

SAMPLY_PROFILE_OUT="${OUT_DIR}/samply-profile.json"
if [[ "${SAMPLY_SAVE_ONLY}" == "1" ]]; then
    FULL_CMD=(samply record --save-only -o "${SAMPLY_PROFILE_OUT}" -- "${REPLAY_CMD[@]}")
else
    FULL_CMD=(samply record -o "${SAMPLY_PROFILE_OUT}" -- "${REPLAY_CMD[@]}")
fi

# Save exact invocation
printf '%s\n' "${FULL_CMD[*]}" > "${OUT_DIR}/command.txt"

# ---------------------------------------------------------------------------
# Collect host metadata
# ---------------------------------------------------------------------------
GIT_SHA=$(git rev-parse HEAD 2>/dev/null || echo "unknown")
GIT_BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
HOST_NAME=$(hostname)
HOST_UNAME=$(uname -srm)

if [[ "$(uname)" == "Linux" ]]; then
    HOST_CPU=$(lscpu 2>/dev/null | grep 'Model name' | sed 's/.*:\s*//' || echo "unknown")
    HOST_MEM_KIB=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo "0")
else
    HOST_CPU=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "unknown")
    MEM_BYTES=$(sysctl -n hw.memsize 2>/dev/null || echo "0")
    HOST_MEM_KIB=$(( MEM_BYTES / 1024 ))
fi

# ---------------------------------------------------------------------------
# Write run_metadata.json
# ---------------------------------------------------------------------------
# Escape double quotes in values that could contain them
esc() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

printf '%s\n' "{
  \"schema_version\": 1,
  \"timestamp_utc\": \"${TIMESTAMP}\",
  \"tool\": \"${TOOL}\",
  \"git_sha\": \"$(esc "${GIT_SHA}")\",
  \"git_branch\": \"$(esc "${GIT_BRANCH}")\",
  \"dataset_path\": \"$(esc "${DATASET}")\",
  \"dataset_manifest_sha256\": \"${MANIFEST_SHA256}\",
  \"backend\": \"${BACKEND}\",
  \"db_dir\": \"$(esc "${RUN_DB_DIR}")\",
  \"keep_db\": ${KEEP_DB},
  \"build\": {
    \"profile\": \"${SNAP_CARGO_PROFILE}\",
    \"features\": \"${FEATURES}\",
    \"rustflags\": \"$(esc "${BUILD_RUSTFLAGS}")\",
    \"binary\": \"$(esc "${BINARY}")\"
  },
  \"host\": {
    \"hostname\": \"$(esc "${HOST_NAME}")\",
    \"uname\": \"$(esc "${HOST_UNAME}")\",
    \"cpu\": \"$(esc "${HOST_CPU}")\",
    \"mem_total_kib\": ${HOST_MEM_KIB}
  },
  \"command\": \"$(esc "${FULL_CMD[*]}")\"
}" > "${OUT_DIR}/run_metadata.json"

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------
echo "==> Running: ${FULL_CMD[*]}"
echo "==> Output dir: ${OUT_DIR}"

"${FULL_CMD[@]}" 2>&1 | tee "${OUT_DIR}/run.log"

echo "==> Done. Artifacts in ${OUT_DIR}"

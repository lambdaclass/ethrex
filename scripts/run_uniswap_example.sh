#!/usr/bin/env bash
set -euo pipefail

LOG_PREFIX="[ethrex-uniswap-workflow]"
log() { echo "${LOG_PREFIX} $*"; }

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
L2_DIR="${ROOT_DIR}/crates/l2"

# Where to find the ethrex-l2-contracts-kit repo.
# Can be overridden with ETHREX_L2_CONTRACTS_KIT_DIR.
KIT_DIR_DEFAULT="${ROOT_DIR}/../ethrex-l2-contracts-kit"
KIT_DIR="${ETHREX_L2_CONTRACTS_KIT_DIR:-${KIT_DIR_DEFAULT}}"

L1_LOG_FILE="${ROOT_DIR}/l1.log"
L2_LOG_FILE="${ROOT_DIR}/l2.log"

L1_RPC_URL="${L1_RPC_URL:-http://127.0.0.1:8545}"
L2_RPC_URL="${L2_RPC_URL:-http://127.0.0.1:1729}"

# Rich L2 account whose balance we wait for (matches the Uniswap example defaults).
RICH_L2_ADDRESS_DEFAULT="0x0000bd19F707CA481886244bDd20Bd6B8a81bd3e"
RICH_L2_ADDRESS="${RICH_ADDRESS:-${RICH_L2_ADDRESS_DEFAULT}}"

L1_PID=""
L2_PID=""

cleanup() {
  local status=$?
  set +e

  if [[ -n "${L1_PID}" ]] && kill -0 "${L1_PID}" 2>/dev/null; then
    log "Stopping L1 node (PID ${L1_PID})..."
    kill "${L1_PID}" 2>/dev/null || true
  fi

  if [[ -n "${L2_PID}" ]] && kill -0 "${L2_PID}" 2>/dev/null; then
    log "Stopping L2 node (PID ${L2_PID})..."
    kill "${L2_PID}" 2>/dev/null || true
  fi

  exit "${status}"
}
trap cleanup EXIT

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    log "Required command '$1' not found in PATH"
    exit 1
  fi
}

wait_for_http() {
  local url="$1"
  local name="$2"
  local max_tries="${3:-60}"
  local sleep_secs="${4:-2}"

  if ! command -v curl >/dev/null 2>&1; then
    log "curl not available; sleeping 10s before continuing with ${name}..."
    sleep 10
    return 0
  fi

  log "Waiting for ${name} at ${url}..."
  local i
  for ((i = 1; i <= max_tries; i++)); do
    if curl -s -o /dev/null "${url}"; then
      log "${name} is up (reachable at ${url})."
      return 0
    fi
    sleep "${sleep_secs}"
  done

  log "Timed out waiting for ${name} at ${url}"
  return 1
}

main() {
  require_cmd make
  require_cmd git
  require_cmd rex

  log "Root dir: ${ROOT_DIR}"
  log "L2 dir:   ${L2_DIR}"
  log "Kit dir:  ${KIT_DIR}"

  if [[ ! -d "${L2_DIR}" ]]; then
    log "L2 directory not found at ${L2_DIR}"
    exit 1
  fi

  # Always start from a clean L1 DB.
  log "Removing L1 DB with 'make rm-db-l1'..."
  (
    cd "${L2_DIR}"
    make rm-db-l1
  )

  # ---------------------------------------------------------------------------
  # Step 1: Start L1 node (background)
  # ---------------------------------------------------------------------------
  log "Starting L1 node with 'make init-l1' (logs: ${L1_LOG_FILE})..."
  (
    cd "${L2_DIR}"
    make init-l1 >"${L1_LOG_FILE}" 2>&1
  ) &
  L1_PID=$!

  # Give L1 a bit of time and wait until it's reachable.
  if ! wait_for_http "${L1_RPC_URL}" "L1 RPC"; then
    log "L1 failed to become ready; see ${L1_LOG_FILE}"
    exit 1
  fi

  # Optional extra delay before deploying, in case L1 needs more time.
  L1_DEPLOY_DELAY="${L1_DEPLOY_DELAY:-5}"
  if [[ "${L1_DEPLOY_DELAY}" -gt 0 ]]; then
    log "Sleeping ${L1_DEPLOY_DELAY}s before deploying L1 contracts..."
    sleep "${L1_DEPLOY_DELAY}"
  fi

  # Ensure the L1 process is still alive after waiting.
  if ! kill -0 "${L1_PID}" 2>/dev/null; then
    log "L1 process exited unexpectedly; see ${L1_LOG_FILE}"
    exit 1
  fi

  # Always start from a clean L2 DB before deploying contracts.
  log "Removing L2 DB with 'make rm-db-l2'..."
  (
    cd "${L2_DIR}"
    make rm-db-l2
  )

  # ---------------------------------------------------------------------------
  # Step 2: Deploy L1 contracts and start L2 node
  # ---------------------------------------------------------------------------
  log "Deploying L1 contracts with 'make deploy-l1'..."
  (
    cd "${L2_DIR}"
    make deploy-l1
  )

  log "Starting L2 node with 'make init-l2' (logs: ${L2_LOG_FILE})..."
  (
    cd "${L2_DIR}"
    make init-l2 >"${L2_LOG_FILE}" 2>&1
  ) &
  L2_PID=$!

  if ! wait_for_http "${L2_RPC_URL}" "L2 RPC"; then
    log "L2 failed to become ready; see ${L2_LOG_FILE}"
    exit 1
  fi

  if ! kill -0 "${L2_PID}" 2>/dev/null; then
    log "L2 process exited unexpectedly; see ${L2_LOG_FILE}"
    exit 1
  fi

  # ---------------------------------------------------------------------------
  # Wait until the rich L2 account has a non-zero balance
  # ---------------------------------------------------------------------------
  log "Waiting for rich L2 account ${RICH_L2_ADDRESS} to have a non-zero balance..."
  MAX_BALANCE_TRIES="${MAX_BALANCE_TRIES:-300}"   # ~10 minutes with 2s sleep
  BALANCE_SLEEP_SECS="${BALANCE_SLEEP_SECS:-2}"

  balance_output=""
  success=false
  for ((i = 1; i <= MAX_BALANCE_TRIES; i++)); do
    if ! balance_output=$(RPC_URL="${L2_RPC_URL}" rex balance "${RICH_L2_ADDRESS}" 2>/dev/null); then
      sleep "${BALANCE_SLEEP_SECS}"
      continue
    fi
    # Extract the last integer from the output (assumed to be the balance in wei).
    balance_wei=$(echo "${balance_output}" | grep -Eo '[0-9]+' | tail -n1 || echo "0")
    if [[ "${balance_wei}" =~ ^[0-9]+$ ]] && (( balance_wei > 0 )); then
      log "Detected L2 balance for ${RICH_L2_ADDRESS}: ${balance_wei} wei"
      success=true
      break
    fi
    sleep "${BALANCE_SLEEP_SECS}"
  done

  if [[ "${success}" != "true" ]]; then
    log "Timed out waiting for non-zero L2 balance for ${RICH_L2_ADDRESS}."
    log "Last rex balance output:"
    echo "${balance_output:-<none>}"
    exit 1
  fi

  # ---------------------------------------------------------------------------
  # Step 3: Run Uniswap example script from ethrex-l2-contracts-kit
  # ---------------------------------------------------------------------------
  log "Ensuring ethrex-l2-contracts-kit is available at ${KIT_DIR}..."

  if [[ ! -d "${KIT_DIR}" ]]; then
    log "Cloning ethrex-l2-contracts-kit (branch add_uniswap_script)..."
    git clone --branch add_uniswap_script \
      https://github.com/lambdaclass/ethrex-l2-contracts-kit.git \
      "${KIT_DIR}"
  else
    log "Repository already exists at ${KIT_DIR}; ensuring branch add_uniswap_script..."
    (
      cd "${KIT_DIR}"
      git fetch origin add_uniswap_script || true
      git checkout add_uniswap_script
    )
  fi

  if [[ ! -x "${KIT_DIR}/examples/uniswap/run.sh" ]]; then
    chmod +x "${KIT_DIR}/examples/uniswap/run.sh" || true
  fi

  log "Running Uniswap example script (examples/uniswap/run.sh)..."
  (
    cd "${KIT_DIR}"
    # Use existing RPC_URL if set, otherwise default to L2_RPC_URL.
    RPC_URL="${RPC_URL:-${L2_RPC_URL}}" \
      examples/uniswap/run.sh
  )

  log "Uniswap example completed successfully."
}

main "$@"

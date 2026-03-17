#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# ZK-DEX Localnet Setup Script
#
# Starts a full ZK-DEX E2E environment: L1 -> Deploy -> L2 -> Prover
# Usage: ./scripts/zk-dex-localnet.sh [start|stop|status|logs] [options]
#   --no-prover    Start L1+L2 only (skip prover, useful for app testing)
# =============================================================================

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
L2_DIR="$REPO_ROOT/crates/l2"
RUNDIR="$L2_DIR/.zk-dex-localnet"

# Network
L1_RPC_URL="http://localhost:8545"
L1_PORT=8545
L1_AUTH_PORT=8551
L2_PORT=1729
L2_PROMETHEUS_METRICS_PORT=3702
PROOF_COORDINATOR_PORT=3900

# Keys
L1_PRIVATE_KEY="0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924"
L2_OWNER_ADDRESS="0x4417092b70a3e5f10dc504d0947dd256b965fc62"
BRIDGE_OWNER_PRIVATE_KEY="0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e"

# Paths
L1_GENESIS="$REPO_ROOT/fixtures/genesis/l1.json"
L2_GENESIS="$REPO_ROOT/fixtures/genesis/l2-zk-dex.json"
ENV_FILE="$REPO_ROOT/cmd/.env"
L1_DB="$RUNDIR/dev_ethrex_l1"
L2_DB="$RUNDIR/dev_ethrex_l2"
PROGRAMS_CONFIG="$L2_DIR/programs-zk-dex.toml"
OSAKA_TIME=1761677592

# =============================================================================
# Helpers
# =============================================================================

log_info() {
    echo "[INFO] $*"
}

log_error() {
    echo "[ERROR] $*" >&2
}

log_step() {
    echo ""
    echo ">>> Step $1: $2"
    echo "-----------------------------------------------------------"
}

is_running() {
    local pidfile="$1"
    if [[ -f "$pidfile" ]]; then
        local pid
        pid=$(cat "$pidfile")
        if kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
    fi
    return 1
}

any_running() {
    is_running "$RUNDIR/l1.pid" || \
    is_running "$RUNDIR/l2.pid" || \
    is_running "$RUNDIR/prover.pid"
}

cleanup() {
    log_info "Cleaning up..."
    do_stop
}

wait_for_rpc() {
    local url="$1"
    local name="$2"
    local timeout="$3"
    local elapsed=0

    log_info "Waiting for $name to be ready at $url (timeout: ${timeout}s)..."
    while ! curl -sf -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$url" > /dev/null 2>&1; do
        sleep 1
        elapsed=$((elapsed + 1))
        if [[ $elapsed -ge $timeout ]]; then
            log_error "$name failed to start within ${timeout}s"
            return 1
        fi
    done
    log_info "$name is ready (took ${elapsed}s)"
}

# =============================================================================
# Commands
# =============================================================================

do_start() {
    local no_prover=false

    for arg in "$@"; do
        case "$arg" in
            --no-prover) no_prover=true ;;
        esac
    done

    # Step 1: Initialize
    log_step 1 "Initializing"

    if any_running; then
        log_error "ZK-DEX localnet is already running. Run 'stop' first."
        exit 1
    fi

    if [[ ! -f "$L2_GENESIS" ]]; then
        log_error "L2 genesis not found: $L2_GENESIS"
        log_error "Run 'scripts/generate-zk-dex-genesis.sh' first to generate the ZK-DEX genesis."
        exit 1
    fi

    mkdir -p "$RUNDIR"
    rm -rf "$L1_DB" "$L2_DB"
    # Clean stale PID files
    rm -f "$RUNDIR"/*.pid
    log_info "Run directory: $RUNDIR"

    # Step 2: Start L1
    log_step 2 "Starting L1"

    cargo run --release --manifest-path "$REPO_ROOT/Cargo.toml" --bin ethrex -- \
        --network "$L1_GENESIS" \
        --http.port $L1_PORT \
        --http.addr 0.0.0.0 \
        --authrpc.port $L1_AUTH_PORT \
        --dev \
        --datadir "$L1_DB" \
        > "$RUNDIR/l1.log" 2>&1 &
    echo $! > "$RUNDIR/l1.pid"
    log_info "L1 started (PID: $(cat "$RUNDIR/l1.pid"))"

    # Step 3: Wait for L1
    log_step 3 "Waiting for L1"

    if ! wait_for_rpc "$L1_RPC_URL" "L1" 300; then
        cleanup
        exit 1
    fi

    # Step 4: Deploy contracts
    log_step 4 "Deploying contracts (L1 + SP1 verifier + ZK-DEX)"

    if ! COMPILE_CONTRACTS=true \
        GUEST_PROGRAMS=evm-l2,zk-dex \
        cargo run --release --features l2,l2-sql,sp1 --manifest-path "$REPO_ROOT/Cargo.toml" -- \
        l2 deploy \
        --eth-rpc-url $L1_RPC_URL \
        --private-key $L1_PRIVATE_KEY \
        --sp1 true \
        --on-chain-proposer-owner $L2_OWNER_ADDRESS \
        --bridge-owner $L2_OWNER_ADDRESS \
        --bridge-owner-pk $BRIDGE_OWNER_PRIVATE_KEY \
        --deposit-rich \
        --private-keys-file-path "$REPO_ROOT/fixtures/keys/private_keys_l1.txt" \
        --genesis-l1-path "$L1_GENESIS" \
        --genesis-l2-path "$L2_GENESIS" \
        --register-guest-programs zk-dex \
        2>&1 | tee "$RUNDIR/deploy.log"; then
        log_error "Contract deployment failed"
        cleanup
        exit 1
    fi

    # Step 5: Start L2
    log_step 5 "Starting L2"

    # Load deployed contract addresses from .env
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a

    GUEST_PROGRAMS=evm-l2,zk-dex \
    ETHREX_GUEST_PROGRAM_ID=zk-dex \
    cargo run --release --features l2,l2-sql,sp1 --manifest-path "$REPO_ROOT/Cargo.toml" -- \
        l2 \
        --no-monitor \
        --proof-coordinator.guest-program-id zk-dex \
        --watcher.block-delay 0 \
        --network "$L2_GENESIS" \
        --http.port $L2_PORT \
        --http.addr 0.0.0.0 \
        --metrics --metrics.port $L2_PROMETHEUS_METRICS_PORT \
        --datadir "$L2_DB" \
        --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
        --l1.on-chain-proposer-address "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" \
        --eth.rpc-url $L1_RPC_URL \
        --osaka-activation-time $OSAKA_TIME \
        --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
        --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
        --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
        --block-producer.operator-fee-per-gas 1000000000 \
        --committer.l1-private-key $L1_PRIVATE_KEY \
        --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
        --proof-coordinator.addr 127.0.0.1 \
        --p2p.port 30304 \
        --discovery.port 30304 \
        > "$RUNDIR/l2.log" 2>&1 &
    echo $! > "$RUNDIR/l2.pid"
    log_info "L2 started (PID: $(cat "$RUNDIR/l2.pid"))"

    # Step 6: Wait for L2
    log_step 6 "Waiting for L2"

    if ! wait_for_rpc "http://localhost:$L2_PORT" "L2" 600; then
        cleanup
        exit 1
    fi

    # Step 7: Start Prover (unless --no-prover)
    local prover_status="skipped (--no-prover)"
    if [[ "$no_prover" == false ]]; then
        log_step 7 "Starting SP1 Prover"

        GUEST_PROGRAMS=evm-l2,zk-dex \
        cargo run --release --features l2,l2-sql,sp1 --manifest-path "$REPO_ROOT/Cargo.toml" -- \
            l2 prover \
            --proof-coordinators tcp://127.0.0.1:$PROOF_COORDINATOR_PORT \
            --backend sp1 \
            --programs-config "$PROGRAMS_CONFIG" \
            > "$RUNDIR/prover.log" 2>&1 &
        echo $! > "$RUNDIR/prover.pid"
        prover_status="running (PID: $(cat "$RUNDIR/prover.pid"))"
        log_info "Prover started (PID: $(cat "$RUNDIR/prover.pid"))"
    else
        log_step 7 "Skipping Prover (--no-prover)"
    fi

    # Step 8: Print summary
    local on_chain_proposer="${ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS:-N/A}"
    local bridge="${ETHREX_WATCHER_BRIDGE_ADDRESS:-N/A}"
    local sp1_verifier="${ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS:-N/A}"
    local timelock="${ETHREX_TIMELOCK_ADDRESS:-N/A}"

    printf '\n'
    printf '========================================\n'
    printf '  ZK-DEX Localnet is running!\n'
    printf '========================================\n'
    printf '  L1 RPC:     http://localhost:%s\n' "$L1_PORT"
    printf '  L2 RPC:     http://localhost:%s\n' "$L2_PORT"
    printf '  Prover:     %s\n' "$prover_status"
    printf '\n'
    printf '  Contract Addresses:\n'
    printf '    OnChainProposer: %s\n' "$on_chain_proposer"
    printf '    Bridge:          %s\n' "$bridge"
    printf '    SP1 Verifier:    %s\n' "$sp1_verifier"
    printf '    Timelock:        %s\n' "$timelock"
    printf '\n'
    printf '  Logs:   %s/*.log\n' "$RUNDIR"
    printf '  Stop:   ./scripts/zk-dex-localnet.sh stop\n'
    printf '========================================\n'
}

do_stop() {
    log_info "Stopping ZK-DEX localnet..."

    # Stop in reverse order: prover -> l2 -> l1
    for component in prover l2 l1; do
        local pidfile="$RUNDIR/${component}.pid"
        if [[ ! -f "$pidfile" ]]; then
            continue
        fi

        local pid
        pid=$(cat "$pidfile")

        if kill -0 "$pid" 2>/dev/null; then
            log_info "Stopping $component (PID: $pid)..."
            kill -INT "$pid" 2>/dev/null || true

            # Wait up to 10 seconds for graceful shutdown
            local elapsed=0
            while kill -0 "$pid" 2>/dev/null && [[ $elapsed -lt 10 ]]; do
                sleep 1
                elapsed=$((elapsed + 1))
            done

            # Force kill if still running
            if kill -0 "$pid" 2>/dev/null; then
                log_info "Force killing $component (PID: $pid)..."
                kill -9 "$pid" 2>/dev/null || true
            fi
        fi

        rm -f "$pidfile"
    done

    log_info "ZK-DEX localnet stopped."
}

do_status() {
    echo "ZK-DEX Localnet Status"
    echo "-----------------------------------------------------------"

    for component in l1 l2 prover; do
        local pidfile="$RUNDIR/${component}.pid"
        local label
        label=$(echo "$component" | tr '[:lower:]' '[:upper:]')

        if [[ ! -f "$pidfile" ]]; then
            echo "  $label: not running"
            continue
        fi

        local pid
        pid=$(cat "$pidfile")
        if kill -0 "$pid" 2>/dev/null; then
            echo "  $label: running (PID: $pid)"
        else
            echo "  $label: dead (stale pid: $pid)"
        fi
    done
}

do_logs() {
    local component="${1:-}"

    if [[ -n "$component" ]]; then
        local logfile="$RUNDIR/${component}.log"
        if [[ ! -f "$logfile" ]]; then
            log_error "Log file not found: $logfile"
            exit 1
        fi
        tail -f "$logfile"
    else
        # Follow all available logs
        local logfiles=()
        for f in "$RUNDIR"/*.log; do
            [[ -f "$f" ]] && logfiles+=("$f")
        done

        if [[ ${#logfiles[@]} -eq 0 ]]; then
            log_error "No log files found in $RUNDIR"
            exit 1
        fi
        tail -f "${logfiles[@]}"
    fi
}

# =============================================================================
# Main
# =============================================================================

COMMAND="${1:-start}"
shift || true

case "$COMMAND" in
    start)  do_start "$@" ;;
    stop)   do_stop ;;
    status) do_status ;;
    logs)   do_logs "$@" ;;
    *)
        echo "Usage: $0 [start|stop|status|logs] [--no-prover]"
        echo ""
        echo "Commands:"
        echo "  start        Start the full ZK-DEX localnet"
        echo "  stop         Stop all running components"
        echo "  status       Show status of each component"
        echo "  logs [name]  Tail logs (optionally: l1, l2, prover, deploy)"
        echo ""
        echo "Options:"
        echo "  --no-prover  Skip starting the prover (app testing)"
        exit 1
        ;;
esac

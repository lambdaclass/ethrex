#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# ZK-DEX Docker Localnet Setup Script
#
# Starts a full ZK-DEX E2E environment using Docker Compose:
#   L1 -> Contract Deploy (SP1 verifier) -> L2 (ZK-DEX) -> Prover (SP1 + GPU)
#
# Usage: ./scripts/zk-dex-docker.sh [start|stop|status|logs|clean] [options]
#   --no-prover    Start L1+L2 only (skip prover)
#   --no-gpu       Disable GPU acceleration (CPU-only proving)
#   --no-build     Skip rebuilding Docker images
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
L2_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$L2_DIR/../.." && pwd)"

# Docker Compose files
COMPOSE_BASE="$L2_DIR/docker-compose.yaml"
COMPOSE_ZK_DEX="$L2_DIR/docker-compose-zk-dex.overrides.yaml"
COMPOSE_GPU="$L2_DIR/docker-compose-zk-dex-gpu.overrides.yaml"
COMPOSE_TOOLS="$L2_DIR/docker-compose-zk-dex-tools.yaml"

# Required env for Docker Compose
export DOCKER_ETHREX_WORKDIR=/usr/local/bin

# Ports (for health checks)
L1_PORT=8545
L2_PORT=1729

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

compose_cmd() {
    local -a compose_files=("-f" "$COMPOSE_BASE" "-f" "$COMPOSE_ZK_DEX")

    # Add GPU override only when GPU is available and not disabled
    if [[ "${ZK_DEX_USE_GPU:-false}" == "true" ]]; then
        compose_files+=("-f" "$COMPOSE_GPU")
    fi

    docker compose "${compose_files[@]}" "$@"
}

check_gpu() {
    # Check if NVIDIA GPU is available
    if command -v nvidia-smi &> /dev/null && nvidia-smi &> /dev/null; then
        return 0
    fi
    return 1
}

wait_for_container_rpc() {
    local url="$1"
    local name="$2"
    local timeout="$3"
    local elapsed=0

    log_info "Waiting for $name at $url (timeout: ${timeout}s)..."
    while ! curl -sf -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$url" > /dev/null 2>&1; do
        sleep 2
        elapsed=$((elapsed + 2))
        if [[ $elapsed -ge $timeout ]]; then
            log_error "$name failed to start within ${timeout}s"
            log_error "Check logs: $0 logs"
            return 1
        fi
    done
    log_info "$name is ready (took ${elapsed}s)"
}

wait_for_container_exit() {
    local container="$1"
    local timeout="$2"
    local elapsed=0

    log_info "Waiting for $container to complete (timeout: ${timeout}s)..."
    while true; do
        local state
        state=$(docker inspect --format='{{.State.Status}}' "$container" 2>/dev/null || echo "not_found")

        case "$state" in
            exited)
                local exit_code
                exit_code=$(docker inspect --format='{{.State.ExitCode}}' "$container" 2>/dev/null || echo "1")
                if [[ "$exit_code" == "0" ]]; then
                    log_info "$container completed successfully"
                    return 0
                else
                    log_error "$container failed with exit code $exit_code"
                    log_error "Logs:"
                    docker logs --tail 30 "$container" 2>&1 || true
                    return 1
                fi
                ;;
            running)
                ;;
            not_found)
                # Container might not be created yet, wait a bit
                if [[ $elapsed -gt 10 ]]; then
                    log_error "Container $container not found"
                    return 1
                fi
                ;;
        esac

        sleep 2
        elapsed=$((elapsed + 2))
        if [[ $elapsed -ge $timeout ]]; then
            log_error "$container did not finish within ${timeout}s"
            return 1
        fi
    done
}

# =============================================================================
# Commands
# =============================================================================

do_start() {
    local no_prover=false
    local no_gpu=false
    local build_flag="--build"

    for arg in "$@"; do
        case "$arg" in
            --no-prover) no_prover=true ;;
            --no-gpu)    no_gpu=true ;;
            --no-build)  build_flag="" ;;
        esac
    done

    # Pre-flight checks
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed."
        exit 1
    fi

    if ! docker info &> /dev/null; then
        log_error "Docker daemon is not running. Start Docker first."
        exit 1
    fi

    # GPU detection: opt-in only when GPU is available and not disabled
    if [[ "$no_gpu" == true ]]; then
        export ZK_DEX_USE_GPU=false
        log_info "GPU disabled by --no-gpu flag, using CPU-only mode"
    elif check_gpu; then
        export ZK_DEX_USE_GPU=true
        log_info "NVIDIA GPU detected, GPU acceleration enabled"
    else
        export ZK_DEX_USE_GPU=false
        log_info "No NVIDIA GPU detected, using CPU-only mode"
    fi

    # Ensure .env file exists (needed by docker compose volume mount)
    touch "$L2_DIR/.env"

    # Step 1: Clean previous state
    log_step 1 "Cleaning previous state"
    compose_cmd down --volumes --remove-orphans 2>/dev/null || true
    log_info "Previous containers cleaned"

    # Step 2: Build SP1 Docker image (first time takes a while)
    log_step 2 "Building SP1 Docker images"
    if [[ -n "$build_flag" ]]; then
        log_info "Building ethrex:sp1 image (includes SP1 toolchain + ZK-DEX guest programs)..."
        log_info "This may take 10-20 minutes on first build (cached on subsequent runs)"
        compose_cmd build
    else
        log_info "Skipping build (--no-build)"
    fi

    # Step 3: Start L1
    log_step 3 "Starting L1 (tokamak-app-l1)"
    compose_cmd up -d tokamak-app-l1

    if ! wait_for_container_rpc "http://localhost:$L1_PORT" "L1" 120; then
        log_error "L1 failed to start. Check: $0 logs l1"
        do_stop
        exit 1
    fi

    # Step 4: Deploy contracts (SP1 verifier + ZK-DEX guest program registration)
    log_step 4 "Deploying contracts (SP1 verifier + ZK-DEX)"
    compose_cmd up -d tokamak-app-deployer

    if ! wait_for_container_exit "tokamak-app-deployer" 300; then
        log_error "Contract deployment failed. Check: $0 logs deploy"
        do_stop
        exit 1
    fi

    # Step 5: Start L2 with ZK-DEX guest program
    # NOTE: depends_on causes tokamak-app-deployer to re-run, which may
    # redeploy contracts with new addresses. We extract addresses AFTER
    # L2 starts to get the final deployed addresses.
    log_step 5 "Starting L2 (ZK-DEX guest program)"
    compose_cmd up -d tokamak-app-l2

    if ! wait_for_container_rpc "http://localhost:$L2_PORT" "L2" 120; then
        log_error "L2 failed to start. Check: $0 logs l2"
        do_stop
        exit 1
    fi

    # Extract deployed contract addresses AFTER L2 is up
    # (deployer may run again due to depends_on, so we get the final addresses)
    log_info "Extracting deployed contract addresses..."
    docker exec tokamak-app-l2 cat /env/.env > "$L2_DIR/.zk-dex-deployed.env" 2>/dev/null || \
        docker cp tokamak-app-deployer:/env/.env "$L2_DIR/.zk-dex-deployed.env" 2>/dev/null || true
    if [[ -f "$L2_DIR/.zk-dex-deployed.env" ]]; then
        log_info "Deployed addresses saved to .zk-dex-deployed.env"
        grep -E "^ETHREX_WATCHER_BRIDGE_ADDRESS=" "$L2_DIR/.zk-dex-deployed.env" || true
        grep -E "^ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=" "$L2_DIR/.zk-dex-deployed.env" || true
    else
        log_error "Warning: Could not extract deployed addresses. Bridge UI may not work correctly."
    fi

    # Step 6: Start Prover
    local prover_status="skipped (--no-prover)"
    if [[ "$no_prover" == false ]]; then
        local gpu_label="CPU-only"
        if [[ "${ZK_DEX_USE_GPU}" == "true" ]]; then
            gpu_label="GPU accelerated"
        fi
        log_step 6 "Starting SP1 Prover ($gpu_label)"
        compose_cmd up -d tokamak-app-prover
        prover_status="running (SP1, $gpu_label)"
        log_info "Prover started"
    else
        log_step 6 "Skipping Prover (--no-prover)"
    fi

    # Summary
    printf '\n'
    printf '========================================\n'
    printf '  ZK-DEX Docker Localnet is running!\n'
    printf '========================================\n'
    printf '  L1 RPC:      http://localhost:%s\n' "$L1_PORT"
    printf '  L2 RPC:      http://localhost:%s\n' "$L2_PORT"
    printf '  Coordinator:  tcp://127.0.0.1:3900\n'
    printf '  Prover:      %s\n' "$prover_status"
    printf '\n'
    printf '  Commands:\n'
    printf '    Logs:    %s logs [l1|l2|prover|deploy]\n' "$0"
    printf '    Status:  %s status\n' "$0"
    printf '    Stop:    %s stop\n' "$0"
    printf '========================================\n'
}

do_stop() {
    log_info "Stopping ZK-DEX Docker localnet..."
    compose_cmd down --remove-orphans 2>/dev/null || true
    log_info "ZK-DEX Docker localnet stopped."
}

do_status() {
    echo "ZK-DEX Docker Localnet Status"
    echo "-----------------------------------------------------------"
    compose_cmd ps --format "table {{.Name}}\t{{.Status}}\t{{.Ports}}" 2>/dev/null || \
        compose_cmd ps
}

do_logs() {
    local component="${1:-}"

    case "$component" in
        l1)     compose_cmd logs -f tokamak-app-l1 ;;
        l2)     compose_cmd logs -f tokamak-app-l2 ;;
        prover) compose_cmd logs -f tokamak-app-prover ;;
        deploy) compose_cmd logs tokamak-app-deployer ;;
        "")     compose_cmd logs -f ;;
        *)
            log_error "Unknown component: $component"
            echo "Available: l1, l2, prover, deploy"
            exit 1
            ;;
    esac
}

do_clean() {
    log_info "Cleaning all ZK-DEX Docker resources..."
    compose_cmd down --volumes --remove-orphans --rmi local 2>/dev/null || true
    log_info "Docker resources cleaned."
}

# =============================================================================
# Tools Commands (Blockscout, Bridge UI, Dashboard)
# =============================================================================

tools_compose_cmd() {
    docker compose -f "$COMPOSE_TOOLS" "$@"
}

do_tools_start() {
    log_info "Starting ZK-DEX support tools (Blockscout + Bridge UI + Dashboard)..."

    # Check that deployed addresses exist
    if [[ ! -f "$L2_DIR/.zk-dex-deployed.env" ]]; then
        log_error "No deployed addresses found at $L2_DIR/.zk-dex-deployed.env"
        log_error "Run '$0 start' first to deploy contracts and extract addresses."
        exit 1
    fi
    log_info "Using deployed addresses from .zk-dex-deployed.env"

    # Build bridge UI image
    log_info "Building bridge UI image..."
    tools_compose_cmd build

    # Start all tools
    tools_compose_cmd up -d

    printf '\n'
    printf '========================================\n'
    printf '  ZK-DEX Tools are starting!\n'
    printf '========================================\n'
    printf '  Dashboard:      http://localhost:3000\n'
    printf '  Bridge UI:      http://localhost:3000/bridge.html\n'
    printf '  L1 Blockscout:  http://localhost:8083\n'
    printf '  L2 Blockscout:  http://localhost:8082\n'
    printf '\n'
    printf '  Note: Blockscout may take 1-2 minutes to\n'
    printf '  fully start and begin indexing blocks.\n'
    printf '========================================\n'
}

do_tools_stop() {
    log_info "Stopping ZK-DEX support tools..."
    tools_compose_cmd down --remove-orphans 2>/dev/null || true
    log_info "ZK-DEX tools stopped."
}

do_tools_status() {
    echo "ZK-DEX Tools Status"
    echo "-----------------------------------------------------------"
    tools_compose_cmd ps --format "table {{.Name}}\t{{.Status}}\t{{.Ports}}" 2>/dev/null || \
        tools_compose_cmd ps
}

do_tools_clean() {
    log_info "Cleaning ZK-DEX tools Docker resources..."
    tools_compose_cmd down --volumes --remove-orphans --rmi local 2>/dev/null || true
    log_info "ZK-DEX tools Docker resources cleaned."
}

# =============================================================================
# Main
# =============================================================================

COMMAND="${1:-start}"
shift || true

case "$COMMAND" in
    start)        do_start "$@" ;;
    stop)         do_stop ;;
    status)       do_status ;;
    logs)         do_logs "$@" ;;
    clean)        do_clean ;;
    tools-start)  do_tools_start ;;
    tools-stop)   do_tools_stop ;;
    tools-status) do_tools_status ;;
    tools-clean)  do_tools_clean ;;
    *)
        echo "Usage: $0 [command] [options]"
        echo ""
        echo "Localnet Commands:"
        echo "  start        Start the full ZK-DEX localnet with Docker"
        echo "  stop         Stop all containers"
        echo "  status       Show container status"
        echo "  logs [name]  Tail logs (l1, l2, prover, deploy)"
        echo "  clean        Stop and remove all images/volumes"
        echo ""
        echo "Tools Commands (Blockscout, Bridge UI, Dashboard):"
        echo "  tools-start  Start Blockscout + Bridge UI + Dashboard"
        echo "  tools-stop   Stop all tools"
        echo "  tools-status Show tools status"
        echo "  tools-clean  Stop and remove tools images/volumes"
        echo ""
        echo "Options:"
        echo "  --no-prover  Skip starting the prover"
        echo "  --no-gpu     Disable GPU acceleration (CPU-only SP1 proving)"
        echo "  --no-build   Skip rebuilding Docker images"
        exit 1
        ;;
esac

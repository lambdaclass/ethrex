#!/bin/bash
# Master deployment script for EIP-8141 Frame Transactions devnet
# Deploys to ethrex-mainnet-8 via SSH
set -euo pipefail

SERVER="${SERVER:-admin@ethrex-mainnet-8}"
ENCLAVE="eip8141"
CONFIG="./fixtures/networks/eip8141-devnet.yaml"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "============================================"
echo "  EIP-8141 Frame Transactions Devnet Deploy"
echo "============================================"
echo "Server: $SERVER"
echo ""

# ----- Step 0: Stop existing services -----
echo "=== Step 0: Stopping existing services ==="
ssh "$SERVER" bash <<'STOP'
set -x
# Stop Kurtosis enclaves
kurtosis enclave stop --all 2>/dev/null || true
kurtosis enclave rm --all --force 2>/dev/null || true
# Stop standalone Docker containers
docker stop $(docker ps -q) 2>/dev/null || true
# Kill tmux sessions (ethrex/lighthouse from Hoodi sync etc.)
tmux kill-server 2>/dev/null || true
# Stop systemd services if any
sudo systemctl stop ethrex 2>/dev/null || true
sudo systemctl stop lighthouse 2>/dev/null || true
echo "All services stopped."
STOP

# ----- Step 1: Update repo and build image -----
echo ""
echo "=== Step 1: Building Docker image ==="
ssh "$SERVER" bash <<'BUILD'
set -e
cd ~/ethrex
git fetch --all
git checkout eip-8141-devnet
git pull origin eip-8141-devnet || git reset --hard origin/eip-8141-devnet
make build-image
echo "Docker image built: ethrex:local"
BUILD

# ----- Step 2: Start Kurtosis devnet -----
echo ""
echo "=== Step 2: Starting Kurtosis devnet ==="
# Run in background — kurtosis run blocks until the enclave is ready
ssh "$SERVER" bash <<KURTOSIS
set -e
cd ~/ethrex
# Ensure ethereum-package is checked out
make checkout-ethereum-package
# Copy Grafana dashboard
cp metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json ethereum-package/src/grafana/ethrex_l1_perf.json 2>/dev/null || true
# Start the devnet
kurtosis run --enclave $ENCLAVE ethereum-package --args-file $CONFIG
KURTOSIS

# ----- Step 3: Extract ports -----
echo ""
echo "=== Step 3: Extracting published ports ==="
INSPECT=$(ssh "$SERVER" "kurtosis enclave inspect $ENCLAVE 2>/dev/null")
echo "$INSPECT"

echo ""
echo "---------------------------------------------"
echo "IMPORTANT: Extract the following from above:"
echo "  - RPC_PORT: The published HTTP port for el-1-ethrex-lighthouse (32000 range)"
echo "  - DORA_PORT: The published port for dora (34000 range)"
echo ""
echo "Then run the remaining steps manually:"
echo ""
echo "  # Step 4: Deploy contracts (get a pre-funded key from kurtosis enclave inspect)"
echo "  ssh $SERVER 'cd ~/ethrex && bash scripts/eip8141-devnet/deploy-contracts.sh http://localhost:<RPC_PORT> <DEPLOYER_PRIVATE_KEY>'"
echo ""
echo "  # Step 5: Start faucet (use a different pre-funded key)"
echo "  ssh $SERVER 'RPC_PORT=<RPC_PORT> FAUCET_PRIVATE_KEY=<KEY> docker compose -f ~/ethrex/scripts/eip8141-devnet/docker-compose-faucet.yaml up -d'"
echo ""
echo "  # Step 6: Start reverse proxy (optional, for clean URLs)"
echo "  ssh $SERVER 'RPC_PORT=<RPC_PORT> DORA_PORT=<DORA_PORT> caddy run --config ~/ethrex/scripts/eip8141-devnet/Caddyfile --adapter caddyfile &'"
echo ""
echo "Chain ID: 3151908"
echo "============================================"

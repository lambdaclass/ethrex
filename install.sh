#!/bin/sh

# Fail immediately if a command exits with a non-zero status
# and treat unset variables as an error when substituting.
set -e -u

ETHREX_REPOSITORY="https://github.com/lambdaclass/ethrex.git"
OUTPUT_SUBDIR=bin

# Create output directory
mkdir -p $OUTPUT_SUBDIR
INSTALL_DIR=$(cd "$OUTPUT_SUBDIR"; pwd)

# Install ethrex
# We need to specify another branch with a fix
# TODO: remove once the fix is merged into main

# Install ethrex L1
cargo install --locked \
    --root . \
    --git $ETHREX_REPOSITORY ethrex \
    --features dev

# cargo install puts output inside `bin`
mv ./bin/ethrex $INSTALL_DIR/ethrex-l1

# Install ethrex L2
cargo install --locked \
    --root . \
    --git $ETHREX_REPOSITORY ethrex \
    --features l2,rollup_storage_libmdbx,metrics

# cargo install puts output inside `bin`
mv ./bin/ethrex $INSTALL_DIR/ethrex-l2

# SETUP L1 and L2 scripts

curl -sSL -o ./genesis-l1-dev.json https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/test_data/genesis-l1-dev.json

cat >l1 <<EOF
#!/bin/sh
# Starts the ethrex L1

$INSTALL_DIR/ethrex-l1 removedb
$INSTALL_DIR/ethrex-l1 --network genesis-l1-dev.json --dev
EOF

chmod u+x ./l1

# TODO: download genesis-l2.json from a remote location
# TODO: run the L2 contract deployer when starting the L2

cat >>.env <<EOF
# Configuration for the ethrex L2
ETHREX_PROOF_COORDINATOR_DEV_MODE=true
L2_GENESIS_FILE_PATH=genesis-l2.json
L2_PORT=1729
L2_RPC_ADDRESS=0.0.0.0
L2_PROMETHEUS_METRICS_PORT=3702
ethrex_L2_DEV_LIBMDBX=dev_ethrex_l2
DEFAULT_BRIDGE_ADDRESS= # set by the L2 deployer
DEFAULT_ON_CHAIN_PROPOSER_ADDRESS= # set by the L2 deployer
L1_RPC_URL=http://localhost:8545
PROOF_COORDINATOR_ADDRESS=127.0.0.1
EOF

# TODO: move options to a .env file so user can configure them
cat >l2 <<EOF
#!/bin/sh
# Start ethrex L2 with the specified network and HTTP server settings.

export $(cat .env | xargs)

ETHREX_PROOF_COORDINATOR_DEV_MODE=${ETHREX_PROOF_COORDINATOR_DEV_MODE} \
	$INSTALL_DIR/ethrex-l2 l2 init \
	--watcher.block-delay 0 \
	--network ${L2_GENESIS_FILE_PATH} \
	--http.port ${L2_PORT} \
	--http.addr ${L2_RPC_ADDRESS} \
	--metrics \
	--metrics.port ${L2_PROMETHEUS_METRICS_PORT} \
	--evm levm \
	--datadir ${ethrex_L2_DEV_LIBMDBX} \
	--l1.bridge-address ${DEFAULT_BRIDGE_ADDRESS} \
	--l1.on-chain-proposer-address ${DEFAULT_ON_CHAIN_PROPOSER_ADDRESS} \
	--eth.rpc-url ${L1_RPC_URL} \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr ${PROOF_COORDINATOR_ADDRESS}
EOF

chmod u+x ./l2

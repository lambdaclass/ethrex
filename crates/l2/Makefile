.PHONY: help init down restart init-l1-docker down-l1-docker restart-l1-docker \
	init-l1 rm-db-l1 deploy-l1 deploy-l1-sp1 init-l2 \
	init-metrics down-metrics restart-metrics down-l2 restart-l2 rm-db-l2 \
	init-prover-sp1 init-prover-risc0 init-prover-exec build-prover clean-tee clean-tee-contract-deps \
	test integration-test integration-test-gpu state-diff-test

.DEFAULT_GOAL := help

# ==============================================================================
# Top-Level Targets
# ==============================================================================

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

init: init-l1-docker deploy-l1 init-metrics init-l2 ## 🚀 Initializes a localnet with Lambda ethrex client as both L1 and L2

down: down-l1-docker down-l2 down-metrics ## 🛑 Shuts down the localnet
	@echo "✅ localnet successfully stopped"


restart: restart-l1-docker deploy-l1 restart-metrics restart-l2 ## 🔄 Restarts the localnet

# ==============================================================================
# Variables
# ==============================================================================

L2_GENESIS_FILE_PATH=../../fixtures/genesis/l2.json
L1_GENESIS_FILE_PATH=../../fixtures/genesis/l1.json
NATIVE_L2_GENESIS_FILE_PATH=../../fixtures/genesis/native_l2.json
NATIVE_RELAYER_PRIVATE_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

ethrex_PATH=$(shell pwd)/../..
ethrex_BIN_PATH=$(ethrex_PATH)/target/release/ethrex
ethrex_METRICS_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/metrics/docker-compose-metrics.yaml
ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/metrics/docker-compose-metrics-l2.overrides.yaml
DOCKER_ETHREX_WORKDIR := /usr/local/bin

L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
L2_OWNER_ADDRESS=0x4417092b70a3e5f10dc504d0947dd256b965fc62
BRIDGE_OWNER_PRIVATE_KEY=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e

ethrex_L2_DEV_DB?=dev_ethrex_l2
ethrex_L1_DEV_DB=dev_ethrex_l1
L1_PORT=8545
L2_PORT=1729
L1_AUTH_PORT=8551
L1_RPC_ADDRESS=0.0.0.0
L2_RPC_ADDRESS=0.0.0.0
PROOF_COORDINATOR_ADDRESS?=127.0.0.1
ETHREX_BLOCK_PRODUCER_OPERATOR_FEE_PER_GAS?=1000000000
# Timestamp for Osaka activation in the hoodi. Sufficient to start network with Prague for tests.
# Future: initialize network with Osaka activated once supported.
ETHREX_OSAKA_ACTIVATION_TIME?=1761677592

# Matches the ports used by the blockchain/metrics dir
L2_PROMETHEUS_METRICS_PORT = 3702

DEFAULT_BRIDGE_ADDRESS=$$(grep ETHREX_WATCHER_BRIDGE_ADDRESS ../../cmd/.env | cut -d= -f2)
DEFAULT_ON_CHAIN_PROPOSER_ADDRESS=$$(grep ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS ../../cmd/.env | cut -d= -f2)

PROVER_SRC_FILES := $(shell find prover/src -type f)

# ==============================================================================
# L1
# ==============================================================================

init-l1-docker: ## 🚀 Initializes an L1 Lambda ethrex Client with Docker (Used with make init)
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	docker compose up -d ethrex_l1

down-l1-docker: ## 🛑 Shuts down the L1 Lambda ethrex Client
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	docker compose down ethrex_l1

restart-l1-docker: down-l1-docker init-l1-docker ## 🔄 Restarts the L1 Lambda ethrex Client

ifdef NATIVE_ROLLUPS
  NATIVE_ROLLUPS? := native-rollups
else
  NATIVE_ROLLUPS? :=
endif

init-l1: ## 🚀 Initializes an L1 Lambda ethrex Client (set NATIVE_ROLLUPS=1 to enable native-rollups)
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features "$(NATIVE_ROLLUPS?)" -- \
	--network ${L1_GENESIS_FILE_PATH} \
	--http.port ${L1_PORT} \
	--http.addr ${L1_RPC_ADDRESS} \
	--authrpc.port ${L1_AUTH_PORT} \
	--dev \
	--datadir ${ethrex_L1_DEV_DB}

rm-db-l1: ## 🛑 Removes the DB used by the L1
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- removedb --datadir ${ethrex_L1_DEV_DB} --force

ifdef NATIVE_ROLLUPS
deploy-l1: ## 📜 Deploys NativeRollup.sol to L1 (native rollups mode)
	COMPILE_CONTRACTS=true \
	cargo run --release --features l2,l2-sql,native-rollups --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--native-rollups \
	--native-rollups.relayer-pk ${NATIVE_RELAYER_PRIVATE_KEY} \
	--genesis-l2-path ${NATIVE_L2_GENESIS_FILE_PATH} \
	--on-chain-proposer-owner 0x0000000000000000000000000000000000000000 \
	--bridge-owner 0x0000000000000000000000000000000000000000
else
deploy-l1: ## 📜 Deploys the L1 contracts
	COMPILE_CONTRACTS=true \
	cargo run --release --features l2,l2-sql --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--on-chain-proposer-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner-pk ${BRIDGE_OWNER_PRIVATE_KEY} \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ${L1_GENESIS_FILE_PATH} \
	--genesis-l2-path ${L2_GENESIS_FILE_PATH}
endif

## Same as deploy-l1 but deploys the SP1 verifier
deploy-l1-sp1: ## 📜 Deploys the L1 contracts with SP1 verifier
	COMPILE_CONTRACTS=true \
	cargo run --release --features l2,l2-sql --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--sp1 true \
	--on-chain-proposer-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner-pk ${BRIDGE_OWNER_PRIVATE_KEY} \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ${L1_GENESIS_FILE_PATH} \
	--genesis-l2-path ${L2_GENESIS_FILE_PATH}

# ==============================================================================
# L2
# ==============================================================================

ifdef SP1
  SP1? := sp1
else
  SP1? :=
endif

ifdef RISC0
  RISC0? := risc0
else
  RISC0? :=
endif

ifdef NATIVE_ROLLUPS
init-l2: ## 🚀 Initializes a native rollup L2 client
	export $(shell cat ../../cmd/.env | xargs); \
	ETHREX_NO_MONITOR=true \
	cargo run --release --features l2,l2-sql,native-rollups --manifest-path ../../Cargo.toml -- \
	l2 \
	--native-rollups \
	--native-rollups.block-time 10000 \
	--native-rollups.contract-address $${ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS} \
	--native-rollups.relayer-pk ${NATIVE_RELAYER_PRIVATE_KEY} \
	--native-rollups.l1-pk ${L1_PRIVATE_KEY} \
	--network ${NATIVE_L2_GENESIS_FILE_PATH} \
	--http.port ${L2_PORT} \
	--http.addr ${L2_RPC_ADDRESS} \
	--datadir ${ethrex_L2_DEV_DB} \
	--eth.rpc-url ${L1_RPC_URL} \
	--l1.bridge-address 0x0000000000000000000000000000000000000000 \
	--l1.on-chain-proposer-address 0x0000000000000000000000000000000000000000 \
	--block-producer.coinbase-address 0x0000000000000000000000000000000000000000 \
	--committer.l1-private-key ${L1_PRIVATE_KEY} \
	--proof-coordinator.l1-private-key ${L1_PRIVATE_KEY}
else
init-l2: ## 🚀 Initializes an L2 Lambda ethrex Client
	export $(shell cat ../../cmd/.env | xargs); \
	cargo run --release --features l2,l2-sql,$(SP1?),$(RISC0?) --manifest-path ../../Cargo.toml -- \
	l2 \
	--watcher.block-delay 0 \
	--network ${L2_GENESIS_FILE_PATH} \
	--http.port ${L2_PORT} \
	--http.addr ${L2_RPC_ADDRESS} \
	--metrics \
	--metrics.port ${L2_PROMETHEUS_METRICS_PORT} \
	--datadir ${ethrex_L2_DEV_DB} \
	--l1.bridge-address ${DEFAULT_BRIDGE_ADDRESS} \
	--l1.on-chain-proposer-address ${DEFAULT_ON_CHAIN_PROPOSER_ADDRESS} \
	--eth.rpc-url ${L1_RPC_URL} \
	--osaka-activation-time ${ETHREX_OSAKA_ACTIVATION_TIME} \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
	--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
	--block-producer.operator-fee-per-gas ${ETHREX_BLOCK_PRODUCER_OPERATOR_FEE_PER_GAS} \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr ${PROOF_COORDINATOR_ADDRESS}
endif

init-l2-dev: ## 🚀 Initializes an L1 and L2 Lambda ethrex Client
	COMPILE_CONTRACTS=true \
	cargo run --release --features l2,l2-sql --manifest-path ../../Cargo.toml -- \
	l2 --dev --osaka-activation-time ${ETHREX_OSAKA_ACTIVATION_TIME}

init-metrics: ## 🚀 Initializes Grafana and Prometheus with containers
	L1_RPC_URL=${L1_RPC_URL} \
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} up -d

down-metrics: ## 🛑 Shuts down the metrics' containers
	L1_RPC_URL=${L1_RPC_URL} \
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} down

restart-metrics: down-metrics init-metrics ## 🔄 Restarts the metrics' containers

down-l2: ## 🛑 Shuts down the L2 Lambda ethrex Client
	pgrep -a -f "ethrex l2" | awk '!/prover/ {print $1}' | xargs -r kill -s SIGINT

rm-db-l2: ## 🛑 Removes the DB used by the L2
	cargo run --release --features l2,l2-sql --manifest-path ../../Cargo.toml -- l2 removedb --datadir ${ethrex_L2_DEV_DB} --force

restart-l2: down-l2 rm-db-l2 init-l2 ## 🔄 Restarts the L2 Lambda ethrex Client

# ==============================================================================
# Prover
# ==============================================================================

ifdef GPU
  GPU? := gpu
else
  GPU? :=
endif

build-prover-sp1:
	cargo build --release --features "l2,l2-sql,$(GPU?),sp1" --manifest-path ../../Cargo.toml --bin ethrex

build-prover-risc0:
	cargo build --release --features "l2,l2-sql,$(GPU?),risc0" --manifest-path ../../Cargo.toml --bin ethrex

build-prover-exec:
	cargo build --release --features "l2,l2-sql,$(GPU?)" --manifest-path ../../Cargo.toml --bin ethrex

init-prover-sp1:
	cargo run --release --features "l2,l2-sql,$(GPU?),sp1" --manifest-path ../../Cargo.toml -- \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 \
	--backend sp1

init-prover-risc0:
	cargo run --release --features "l2,l2-sql,$(GPU?),risc0" --manifest-path ../../Cargo.toml -- \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 \
	--backend risc0

init-prover-exec:
	cargo run --release --features "l2,l2-sql,$(GPU?)" --manifest-path ../../Cargo.toml -- \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 \
	--backend exec


# ==============================================================================
# TEE
# ==============================================================================

clean-tee: clean-tee-contract-deps ## 🧹 Cleans TEE artifacts
	rm -rf out/

clean-tee-contract-deps: ## 🧹 Cleans the dependencies for the TEE contracts.
	cd tee/contracts; make clean

# ==============================================================================
# Tests
# ==============================================================================

test: ## 🚧 Runs the L2's integration test, run `make init` and in a new terminal make test
	cargo test -p ethrex-test l2_integration_test --profile release-with-debug --features l2 -- --nocapture || (echo "The tests have failed.\n Is the L2 running? To start it, run:\n make rm-db-l1; make rm-db-l2; make restart" ; exit 1)

integration-test: rm-db-l2 rm-db-l1 # We create an empty .env file simply because if the file
	# does not exist, the container fails to write to it.
	touch .env
	docker compose down
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_L2_VALIDIUM=${ETHREX_L2_VALIDIUM} \
	docker compose up --detach --build
	RUST_LOG=info,ethrex_prover_lib=debug make init-prover-exec & \
	ETHREX_WATCHER_BRIDGE_ADDRESS=$(shell make bridge-address) \
	ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=$(shell make on-chain-proposer-address) \
	cargo test -p ethrex-test l2:: --release --features l2 -- --nocapture --test-threads=1
	killall ethrex -s SIGINT

integration-test-gpu: rm-db-l2 rm-db-l1
	# We create an empty .env file simply because if the file
	# does not exists, the container fails to write to it.
	touch .env
	docker compose down
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_BLOCK_PRODUCER_BLOCK_TIME=${ETHREX_BLOCK_PRODUCER_BLOCK_TIME} \
	docker compose up --detach --build

	RUST_LOG=info,ethrex_prover_lib=debug GPU=true make init-prover-sp1 & \
	cargo test -p ethrex-test l2:: --release --features l2 -- --nocapture --test-threads=1
	killall ethrex -s SIGINT # if sent a SIGTERM, SP1 does not shuts down the sp1-gpu container

# State reconstruction tests
# If this fails due to stale blobs, regenerate them following:
# - docs/workflows/regenerate-blobs.md (agent workflow)
# - docs/developers/l2/state-reconstruction-blobs.md (step-by-step guide)
state-diff-test:
	touch .env
	cargo run --release --features l2,l2-sql --manifest-path ../../Cargo.toml -- \
	l2 reconstruct \
	-g ../../fixtures/genesis/l2.json \
	-b ../../fixtures/blobs/ \
	-s $$PWD/store
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_WATCHER_BLOCK_DELAY=0 \
	docker compose -f docker-compose.yaml -f docker-compose-l2-store.overrides.yaml up --detach --no-deps ethrex_l2
	docker logs --follow ethrex_l2 & \
	cargo test -p ethrex-test test_state_reconstruct --release --features l2

# Fast validation to check if blobs match genesis
# Use this before state-diff-test to catch stale blobs early
# If this fails, regenerate blobs following:
# - docs/workflows/regenerate-blobs.md (agent workflow)
# - docs/developers/l2/state-reconstruction-blobs.md (step-by-step guide)
validate-blobs:
	cargo test -p ethrex-test validate_blobs_match_genesis --release

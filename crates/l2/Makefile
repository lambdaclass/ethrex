.PHONY: help init down restart init-l1-docker down-l1-docker restart-l1-docker \
	init-l1 rm-db-l1 deploy-l1 deploy-l1-sp1 init-l2 \
	init-metrics down-metrics restart-metrics down-l2 restart-l2 rm-db-l2 \
	init-prover-sp1 init-prover-risc0 init-prover-exec build-prover clean-tee clean-tee-contract-deps \
	test integration-test integration-test-gpu state-diff-test \
	init-guest-program down-guest-program \
	zk-dex-localnet zk-dex-localnet-no-prover zk-dex-localnet-stop zk-dex-localnet-status \
	zk-dex-docker zk-dex-docker-no-prover zk-dex-docker-no-gpu zk-dex-docker-stop zk-dex-docker-status zk-dex-docker-logs zk-dex-docker-clean \
	zk-dex-docker-tools zk-dex-docker-tools-stop zk-dex-docker-tools-status zk-dex-docker-tools-clean

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
	docker compose up -d tokamak-app-l1

down-l1-docker: ## 🛑 Shuts down the L1 Lambda ethrex Client
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	docker compose down tokamak-app-l1

restart-l1-docker: down-l1-docker init-l1-docker ## 🔄 Restarts the L1 Lambda ethrex Client

init-l1: ## 🚀 Initializes an L1 Lambda ethrex Client
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- \
	--network ${L1_GENESIS_FILE_PATH} \
	--http.port ${L1_PORT} \
	--http.addr ${L1_RPC_ADDRESS} \
	--authrpc.port ${L1_AUTH_PORT} \
	--dev \
	--datadir ${ethrex_L1_DEV_DB}

rm-db-l1: ## 🛑 Removes the DB used by the L1
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- removedb --datadir ${ethrex_L1_DEV_DB} --force

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
# ZK-DEX E2E
# ==============================================================================

deploy-l1-sp1-zk-dex: ## 📜 Deploys L1 contracts with SP1 verifier + ZK-DEX program
	COMPILE_CONTRACTS=true \
	GUEST_PROGRAMS=evm-l2,zk-dex \
	cargo run --release --features l2,l2-sql,sp1 --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--sp1 true \
	--on-chain-proposer-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner ${L2_OWNER_ADDRESS} \
	--bridge-owner-pk ${BRIDGE_OWNER_PRIVATE_KEY} \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ${L1_GENESIS_FILE_PATH} \
	--genesis-l2-path ${L2_GENESIS_FILE_PATH} \
	--register-guest-programs zk-dex

init-l2-zk-dex: ## 🚀 Initializes L2 with ZK-DEX guest program
	export $(shell cat ../../cmd/.env | xargs); \
	GUEST_PROGRAMS=evm-l2,zk-dex \
	cargo run --release --features l2,l2-sql,sp1 --manifest-path ../../Cargo.toml -- \
	l2 \
	--proof-coordinator.guest-program-id zk-dex \
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

init-prover-sp1-zk-dex: ## 🔐 Starts SP1 prover with ZK-DEX program
	GUEST_PROGRAMS=evm-l2,zk-dex \
	cargo run --release --features "l2,l2-sql,$(GPU?),sp1" --manifest-path ../../Cargo.toml -- \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 \
	--backend sp1 \
	--programs-config programs-zk-dex.toml

zk-dex-localnet: ## 🚀 Start ZK-DEX localnet (L1 + deploy + L2 + prover)
	./scripts/zk-dex-localnet.sh start

zk-dex-localnet-no-prover: ## 🚀 Start ZK-DEX localnet without prover
	./scripts/zk-dex-localnet.sh start --no-prover

zk-dex-localnet-stop: ## 🛑 Stop ZK-DEX localnet
	./scripts/zk-dex-localnet.sh stop

zk-dex-localnet-status: ## 📊 Show ZK-DEX localnet status
	./scripts/zk-dex-localnet.sh status

# ==============================================================================
# ZK-DEX Docker E2E (SP1 + GPU)
# ==============================================================================

zk-dex-docker: ## 🐳 Start ZK-DEX Docker localnet (SP1 + GPU)
	./scripts/zk-dex-docker.sh start

zk-dex-docker-no-prover: ## 🐳 Start ZK-DEX Docker localnet without prover
	./scripts/zk-dex-docker.sh start --no-prover

zk-dex-docker-no-gpu: ## 🐳 Start ZK-DEX Docker localnet (SP1 CPU-only)
	./scripts/zk-dex-docker.sh start --no-gpu

zk-dex-docker-stop: ## 🛑 Stop ZK-DEX Docker localnet
	./scripts/zk-dex-docker.sh stop

zk-dex-docker-status: ## 📊 Show ZK-DEX Docker localnet status
	./scripts/zk-dex-docker.sh status

zk-dex-docker-logs: ## 📋 Tail ZK-DEX Docker localnet logs
	./scripts/zk-dex-docker.sh logs

zk-dex-docker-clean: ## 🧹 Clean ZK-DEX Docker images and volumes
	./scripts/zk-dex-docker.sh clean

# ==============================================================================
# ZK-DEX Docker Tools (Blockscout, Bridge UI, Dashboard)
# ==============================================================================

zk-dex-docker-tools: ## 🔧 Start ZK-DEX tools (Blockscout + Bridge UI + Dashboard)
	./scripts/zk-dex-docker.sh tools-start

zk-dex-docker-tools-stop: ## 🛑 Stop ZK-DEX tools
	./scripts/zk-dex-docker.sh tools-stop

zk-dex-docker-tools-status: ## 📊 Show ZK-DEX tools status
	./scripts/zk-dex-docker.sh tools-status

zk-dex-docker-tools-clean: ## 🧹 Clean ZK-DEX tools Docker resources
	./scripts/zk-dex-docker.sh tools-clean

# ==============================================================================
# Guest Programs
# ==============================================================================

# Default guest program if not specified
PROGRAM ?= evm-l2

init-guest-program: ## 🧩 Starts the localnet with a specific guest program (PROGRAM=zk-dex)
	@echo "Starting L2 with guest program: $(PROGRAM)"
	touch .env
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_GUEST_PROGRAM_ID=$(PROGRAM) \
	docker compose -f docker-compose.yaml -f docker-compose-guest-program.overrides.yaml up --detach --build

down-guest-program: ## 🛑 Stops the guest program localnet
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	docker compose -f docker-compose.yaml -f docker-compose-guest-program.overrides.yaml down

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
	docker cp tokamak-app-deployer:/env/.env ../../cmd/.env
	RUST_LOG=info,ethrex_prover_lib=debug make init-prover-exec & \
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
	docker compose -f docker-compose.yaml -f docker-compose-l2-store.overrides.yaml up --detach --no-deps tokamak-app-l2
	docker logs --follow tokamak-app-l2 & \
	cargo test -p ethrex-test test_state_reconstruct --release --features l2

# Fast validation to check if blobs match genesis
# Use this before state-diff-test to catch stale blobs early
# If this fails, regenerate blobs following:
# - docs/workflows/regenerate-blobs.md (agent workflow)
# - docs/developers/l2/state-reconstruction-blobs.md (step-by-step guide)
validate-blobs:
	cargo test -p ethrex-test validate_blobs_match_genesis --release

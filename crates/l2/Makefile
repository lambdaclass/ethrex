.PHONY: help init down restart init-l1-docker down-l1-docker restart-l1-docker \
	init-l1 rm-db-l1 deploy-l1 deploy-l1-sp1 init-l2 \
	init-metrics down-metrics restart-metrics down-l2 restart-l2 rm-db-l2 \
	init-prover build-prover clean-tee clean-tee-contract-deps \
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
L1_GENESIS_FILE_PATH=../../fixtures/genesis/l1-dev.json

ethrex_PATH=$(shell pwd)/../..
ethrex_BIN_PATH=$(ethrex_PATH)/target/release/ethrex
ethrex_METRICS_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/metrics/docker-compose-metrics.yaml
ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/metrics/docker-compose-metrics-l2.overrides.yaml
DOCKER_ETHREX_WORKDIR := /usr/local/bin

L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

ethrex_L2_DEV_DB?=dev_ethrex_l2
ethrex_L1_DEV_DB=dev_ethrex_l1
L1_PORT=8545
L2_PORT=1729
L1_AUTH_PORT=8551
L1_RPC_ADDRESS=0.0.0.0
L2_RPC_ADDRESS=0.0.0.0
PROOF_COORDINATOR_ADDRESS?=127.0.0.1

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
	cargo run --release --bin ethrex --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--sp1.verifier-address 0x00000000000000000000000000000000000000aa \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
	--aligned.aggregator-address 0x00000000000000000000000000000000000000aa \
	--on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ../../fixtures/genesis/l1-dev.json \
	--genesis-l2-path ../../fixtures/genesis/l2.json

## Same as deploy-l1 but deploys the SP1 verifier
deploy-l1-sp1: ## 📜 Deploys the L1 contracts
	COMPILE_CONTRACTS=true \
	cargo run --release --bin ethrex --manifest-path ../../Cargo.toml -- l2 deploy \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--sp1.deploy-verifier \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
	--aligned.aggregator-address 0x00000000000000000000000000000000000000aa \
	--on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ../../fixtures/genesis/l1-dev.json \
	--genesis-l2-path ../../fixtures/genesis/l2.json

# ==============================================================================
# L2
# ==============================================================================

init-l2: ## 🚀 Initializes an L2 Lambda ethrex Client
	export $(shell cat ../../cmd/.env | xargs); \
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- \
	l2 \
	--watcher.block-delay 0 \
	--network ${L2_GENESIS_FILE_PATH} \
	--http.port ${L2_PORT} \
	--http.addr ${L2_RPC_ADDRESS} \
	--metrics \
	--metrics.port ${L2_PROMETHEUS_METRICS_PORT} \
	--evm levm \
	--datadir ${ethrex_L2_DEV_DB} \
	--l1.bridge-address ${DEFAULT_BRIDGE_ADDRESS} \
	--l1.on-chain-proposer-address ${DEFAULT_ON_CHAIN_PROPOSER_ADDRESS} \
	--eth.rpc-url ${L1_RPC_URL} \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr ${PROOF_COORDINATOR_ADDRESS}

init-l2-dev: ## 🚀 Initializes an L1 and L2 Lambda ethrex Client
	COMPILE_CONTRACTS=true \
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- \
	l2 --dev

init-metrics: ## 🚀 Initializes Grafana and Prometheus with containers
	L1_RPC_URL=${L1_RPC_URL} \
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} up -d

down-metrics: ## 🛑 Shuts down the metrics' containers
	L1_RPC_URL=${L1_RPC_URL} \
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} down

restart-metrics: down-metrics init-metrics ## 🔄 Restarts the metrics' containers

down-l2: ## 🛑 Shuts down the L2 Lambda ethrex Client
	pkill -x ethrex || exit 0

rm-db-l2: ## 🛑 Removes the DB used by the L2
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- l2 removedb --datadir ${ethrex_L2_DEV_DB} --force

restart-l2: down-l2 rm-db-l2 init-l2 ## 🔄 Restarts the L2 Lambda ethrex Client

# ==============================================================================
# Prover
# ==============================================================================

init-prover: ../../target/release/ethrex ## 🚀 Initializes the Prover
	@if [ -z "$$PROVER" ]; then \
		PROVER="exec"; \
	fi; \
	BACKEND="$$PROVER" \
	../../target/release/ethrex l2 prover \
		--backend "$$PROVER" \
		--proof-coordinator tcp://127.0.0.1:3900 \
		--log.level debug

build-prover:
	rm -f ../../target/release/ethrex
	$(MAKE) ../../target/release/ethrex

../../target/release/ethrex: $(PROVER_SRC_FILES)
	@if [ -z "$$PROVER" ]; then \
		echo "Warn: ProverType (PROVER) is missing. Building only with exec mode."; \
		echo "Please provide it as an argument:"; \
		echo "make init-prover PROVER=<prover_type: (risc0, sp1)> <G=true>"; \
		echo "The prover can also be run with GPU (G)"; \
	fi; \

	@if [ -z "$$G" ]; then \
		GPU=""; \
	else \
		GPU=",gpu"; \
	fi; \

	RUSTFLAGS='-C target-cpu=native' \
	cargo build --release \
		--manifest-path ../../Cargo.toml \
		--bin ethrex \
		--features "$$PROVER$$GPU"

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
	cargo test l2_integration_test --profile release-with-debug -- --nocapture || (echo "The tests have failed.\n Is the L2 running? To start it, run:\n make rm-db-l1; make rm-db-l2; make restart" ; exit 1)

integration-test: rm-db-l2 rm-db-l1 # We create an empty .env file simply because if the file
	# does not exist, the container fails to write to it.
	touch .env
	docker compose down
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_L2_VALIDIUM=${ETHREX_L2_VALIDIUM} \
	docker compose up --detach --build
	RUST_LOG=info,ethrex_prover_lib=debug make init-prover & \
	ETHREX_WATCHER_BRIDGE_ADDRESS=$(shell make bridge-address) \
	ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=$(shell make on-chain-proposer-address) \
	cargo test l2 --release -- --nocapture --test-threads=1
	killall ethrex -s SIGINT

integration-test-gpu: rm-db-l2 rm-db-l1
	# We create an empty .env file simply because if the file
	# does not exists, the container fails to write to it.
	touch .env
	docker compose down
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_BLOCK_PRODUCER_BLOCK_TIME=${ETHREX_BLOCK_PRODUCER_BLOCK_TIME} \
	docker compose up --detach --build

	RUST_LOG=info,ethrex_prover_lib=debug SP1_PROVER=cuda make init-prover & \
	cargo test l2 --release -- --nocapture --test-threads=1
	killall ethrex -s SIGINT # if sent a SIGTERM, SP1 does not shuts down the sp1-gpu container

# State reconstruction tests
state-diff-test:
	touch .env
	cargo run --release --manifest-path ../../Cargo.toml -- \
	l2 reconstruct \
	-g ../../fixtures/genesis/l2.json \
	-b ../../fixtures/blobs/ \
	-s $$PWD/store \
	-c 0x0007a881CD95B1484fca47615B64803dad620C8d
	DOCKER_ETHREX_WORKDIR=${DOCKER_ETHREX_WORKDIR} \
	ETHREX_WATCHER_BLOCK_DELAY=0 \
	docker compose -f docker-compose.yaml -f docker-compose-l2-store.overrides.yaml up --detach --no-deps ethrex_l2

	cargo test state_reconstruct --release

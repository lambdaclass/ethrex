.PHONY: help init down clean restart init-local-l1 init-l1 down-local-l1 \
		restart-local-l1 rm-db-l1 clean-contract-deps restart-contract-deps deploy-l1 init-l2 \
		init-l2-no-metrics down-l2 restart-l2 init-prover rm-db-l2 ci_test test \
		init-testnet deploy-l1-testnet restart-testnet

.DEFAULT_GOAL := help

L2_GENESIS_FILE_PATH=../../fixtures/genesis/l2.json
L1_GENESIS_FILE_PATH=../../fixtures/genesis/l1-dev.json

# Basic
help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

init: init-local-l1 update-system-contracts deploy-l1 init-l2 ## 🚀 Initializes a localnet with Lambda ethrex client as both L1 and L2

## Same as init but does not do deposits for rich accounts since that doesn't make sense for deployments to devnets/testnets i.e Sepolia
init-testnet: update-system-contracts deploy-l1-testnet init-l2

down: down-local-l1 down-l2 down-metrics## 🛑 Shuts down the localnet

clean: clean-contract-deps ## 🧹 Cleans the localnet
	rm -rf out/

restart: restart-local-l1 deploy-l1 restart-l2 ## 🔄 Restarts the localnet

## Same as restart but for testnet deployment. The local database is cleaned and the contracts are deployed again.
restart-testnet:
	down-l2 down-metrics rm-db-l2 update-system-contracts deploy-l1-testnet init-l2

# Variables

ethrex_PATH=$(shell pwd)/../..
ethrex_BIN_PATH=$(ethrex_PATH)/target/release/ethrex
ethrex_DEV_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/dev/docker-compose-dev.yaml
ethrex_L2_DOCKER_COMPOSE_PATH=./docker-compose-l2.yaml
ethrex_METRICS_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/metrics/docker-compose-metrics.yaml
ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/metrics/docker-compose-metrics-l2.overrides.yaml
CI_ETHREX_WORKDIR := /usr/local/bin

ethrex_L2_CONTRACTS_PATH=./contracts
L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

ethrex_L2_DEV_LIBMDBX?=dev_ethrex_l2
ethrex_L1_DEV_LIBMDBX=dev_ethrex_l1
L1_PORT=8545
L2_PORT=1729
L1_AUTH_PORT=8551
L1_RPC_ADDRESS=0.0.0.0
L2_RPC_ADDRESS=0.0.0.0
PROOF_COORDINATOR_ADDRESS?=127.0.0.1
ETHREX_PROOF_COORDINATOR_DEV_MODE?=true

# Matches the ports used by the blockchain/metrics dir
L2_PROMETHEUS_METRICS_PORT = 3702

# Local L1
init-local-l1: ## 🚀 Initializes an L1 Lambda ethrex Client with Docker (Used with make init)
	docker compose -f ${ethrex_DEV_DOCKER_COMPOSE_PATH}  up -d

init-l1: ## 🚀 Initializes an L1 Lambda ethrex Client
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features "dev" -- \
	--network ${L1_GENESIS_FILE_PATH} \
	--http.port ${L1_PORT} \
	--http.addr ${L1_RPC_ADDRESS} \
	--authrpc.port ${L1_AUTH_PORT} \
	--dev \
	--datadir ${ethrex_L1_DEV_LIBMDBX}

init-l1-levm: ## 🚀 Initializes an L1 Lambda ethrex Client with LEVM
	cargo run --release \
    --manifest-path ../../Cargo.toml \
    --bin ethrex \
    --features "dev" -- \
    --network ${L1_GENESIS_FILE_PATH} \
    --http.port ${L1_PORT} \
    --http.addr ${L1_RPC_ADDRESS} \
    --authrpc.port ${L1_AUTH_PORT} \
	--evm levm \
	--dev \
    --datadir ${ethrex_L1_DEV_LIBMDBX}

down-local-l1: ## 🛑 Shuts down the L1 Lambda ethrex Client
	docker compose -f ${ethrex_DEV_DOCKER_COMPOSE_PATH} down
	docker compose -f docker-compose-l2.yaml down

restart-local-l1: down-local-l1 rm-db-l1 init-local-l1 ## 🔄 Restarts the L1 Lambda ethrex Client

rm-db-l1: ## 🛑 Removes the DB used by the L1
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- removedb --datadir ${ethrex_L1_DEV_LIBMDBX} --force

# Contracts
clean-contract-deps: ## 🧹 Cleans the dependencies for the L1 contracts.
	rm -rf contracts/solc_out
	rm -rf contracts/lib
	cd tee/contracts; make clean

restart-contract-deps: clean-contract-deps ## 🔄 Restarts the dependencies for the L1 contracts.

deploy-l1: ## 📜 Deploys the L1 contracts
	cargo run --release --bin ethrex_l2_l1_deployer --manifest-path contracts/Cargo.toml -- \
	--eth-rpc-url ${L1_RPC_URL} \
	--private-key ${L1_PRIVATE_KEY} \
	--contracts-path contracts \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--sp1.verifier-address 0x00000000000000000000000000000000000000aa \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
	--aligned.aggregator-address 0x00000000000000000000000000000000000000aa \
	--on-chain-proposer-owner 0x03d0a0aee676cc45bf7032649e0871927c947c8e \
	--bridge-owner 0x03d0a0aee676cc45bf7032649e0871927c947c8e \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ../../fixtures/genesis/l1-dev.json \
	--genesis-l2-path ../../fixtures/genesis/l2.json

## Same as deploy-l1 but does not do deposits for rich accounts since that doesn't make sense for deployments to devnets/testnets i.e Sepolia
deploy-l1-testnet: ## 📜 Deploys the L1 contracts
	cargo run --release --bin ethrex_l2_l1_deployer --manifest-path contracts/Cargo.toml -- \
	--contracts-path contracts

update-system-contracts:
	cargo run --release --bin ethrex_l2_system_contracts_updater --manifest-path contracts/Cargo.toml -- \
	--contracts-path contracts \
	--l2-genesis-path ../../fixtures/genesis/l2.json

# L2
init-l2: init-metrics init-l2-no-metrics ## 🚀 Initializes an L2 Lambda ethrex Client with metrics

DEFAULT_BRIDGE_ADDRESS=$$(grep ETHREX_WATCHER_BRIDGE_ADDRESS .env | cut -d= -f2)
DEFAULT_ON_CHAIN_PROPOSER_ADDRESS=$$(grep ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS .env | cut -d= -f2)

init-l2-no-metrics: ## 🚀 Initializes an L2 Lambda ethrex Client
	export $(shell cat .env | xargs); \
	ETHREX_PROOF_COORDINATOR_DEV_MODE=${ETHREX_PROOF_COORDINATOR_DEV_MODE} \
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features "rollup_storage_sql,metrics" -- \
	l2 init \
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

init-metrics: ## 🚀 Initializes Grafana and Prometheus with containers
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} up -d

down-metrics: ## 🛑 Shuts down the metrics' containers
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} down

down-l2: ## 🛑 Shuts down the L2 Lambda ethrex Client
	pkill -x ethrex || exit 0

restart-l2: down-l2 rm-db-l2 init-l2 ## 🔄 Restarts the L2 Lambda ethrex Client

PROVER_SRC_FILES := $(shell find prover/src -type f)

init-prover: ../../target/release/ethrex_prover ## 🚀 Initializes the Prover
	CONFIGS_PATH=${ethrex_L2_CONFIGS_PATH} \
	../../target/release/ethrex_prover \
	--http.addr 127.0.0.1 \
  	--http.port 3900 \
	--log.level debug

build-prover:
	rm -f ../../target/release/ethrex_prover
	$(MAKE) ../../target/release/ethrex_prover

../../target/release/ethrex_prover: $(PROVER_SRC_FILES)
	@if [ -z "$$PROVER" ]; then \
		echo "Error: ProverType (PROVER) is missing. Running in exec mode."; \
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
	cargo build --release --features "$$PROVER$$GPU,l2" \
	--manifest-path ./prover/Cargo.toml  \
	--bin ethrex_prover

rm-db-l2: ## 🛑 Removes the DB used by the L2
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- l2 removedb --datadir ${ethrex_L2_DEV_LIBMDBX} --force

test: ## 🚧 Runs the L2's integration test, run `make init` and in a new terminal make test
	cargo test l2_integration_test --release -- --nocapture || (echo "The tests have failed.\n Is the L2 running? To start it, run:\n make rm-db-l1; make rm-db-l2; make restart" ; exit 1)

integration-test: rm-db-l2 rm-db-l1# We create an empty .env file simply because if the file
	# does not exist, the container fails to write to it.
	touch .env
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} down
	CI_ETHREX_WORKDIR=${CI_ETHREX_WORKDIR} \
	ETHREX_L2_VALIDIUM=${ETHREX_L2_VALIDIUM} \
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} up --detach --build
	RUST_LOG=info,ethrex_prover_lib=debug make init-prover & \
	ETHREX_WATCHER_BRIDGE_ADDRESS=$(shell make bridge-address) \
	ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=$(shell make on-chain-proposer-address) \
	cargo test l2 --release -- --nocapture --test-threads=1
	killall ethrex_prover -s SIGINT

integration-test-gpu: rm-db-l2 rm-db-l1
	# We create an empty .env file simply because if the file
	# does not exists, the container fails to write to it.
	touch .env
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} down
	CI_ETHREX_WORKDIR=${CI_ETHREX_WORKDIR} \
	ETHREX_PROOF_COORDINATOR_DEV_MODE=${ETHREX_PROOF_COORDINATOR_DEV_MODE} \
	ETHREX_BLOCK_PRODUCER_BLOCK_TIME=${ETHREX_BLOCK_PRODUCER_BLOCK_TIME} \
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} up --detach --build

	RUST_LOG=info,ethrex_prover_lib=debug SP1_PROVER=cuda make init-prover & \
	cargo test l2 --release -- --nocapture --test-threads=1
	killall ethrex_prover -s SIGINT # if sent a SIGTERM, SP1 does not shuts down the sp1-gpu container

# State reconstruction tests
state-diff-test:
	touch .env
	cargo run --release --manifest-path ../../cmd/ethrex/Cargo.toml --bin ethrex -- \
	l2 reconstruct \
	-g ../../fixtures/genesis/l2.json \
	-b ../../fixtures/blobs/ \
	-s $$PWD/store \
	-c 0x0007a881CD95B1484fca47615B64803dad620C8d
	cargo b --manifest-path ../../Cargo.toml --release
	docker compose -f docker-compose-l2.yaml -f docker-compose-l2-store.overrides.yaml down
	CI_ETHREX_WORKDIR=${CI_ETHREX_WORKDIR} \
	ETHREX_WATCHER_BLOCK_DELAY=0 \
	docker compose -f docker-compose-l2.yaml -f docker-compose-l2-store.overrides.yaml up --detach

	cargo test state_reconstruct --release

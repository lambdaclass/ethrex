.PHONY: help init down clean restart cli update-cli-contracts init-local-l1 init-l1 down-local-l1 \
		restart-local-l1 rm-db-l1 clean-contract-deps restart-contract-deps deploy-l1 init-l2 \
		init-l2-no-metrics down-l2 restart-l2 init-prover rm-db-l2 purge_prover_state ci_test test \
		init-testnet deploy-l1-testnet restart-testnet

.DEFAULT_GOAL := help

L2_GENESIS_FILE_PATH=../../test_data/genesis-l2.json
L1_GENESIS_FILE_PATH=../../test_data/genesis-l1-dev.json

# Basic
help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

init: init-local-l1 deploy-l1 update-system-contracts init-l2 ## 🚀 Initializes a localnet with Lambda ethrex client as both L1 and L2

## Same as init but does not do deposits for rich accounts since that doesn't make sense for deployments to devnets/testnets i.e Sepolia
init-testnet: deploy-l1-testnet update-system-contracts  init-l2

down: down-local-l1 down-l2 down-metrics## 🛑 Shuts down the localnet

clean: clean-contract-deps ## 🧹 Cleans the localnet

restart: restart-local-l1 deploy-l1 restart-l2 ## 🔄 Restarts the localnet

## Same as restart but for testnet deployment. The local database is cleaned and the contracts are deployed again.
restart-testnet:
	down-l2 down-metrics rm-db-l2 deploy-l1-testnet update-system-contracts init-l2

# CLI
cli: ## 🛠️ Installs the L2 Lambda ethrex CLI
	cd ${ethrex_PATH}/cmd/ethrex_l2 && \
	cargo install --path . --force

update-cli-contracts: ## 📜 Update the CLI's config contracts
	@if [ -z "$$C" ]; then \
		echo "Error: CONFIG_NAME (C) is missing.\nPlease provide it as an argument:\nmake update-cli-contracts C=<config_name>."; \
		exit 1; \
	fi && \
	CB=$$(grep 'L1_WATCHER_BRIDGE_ADDRESS' .env | cut -d= -f2) && \
	ethrex_l2 config edit --common-bridge $$CB $$C && \
	OP=$$(grep 'COMMITTER_ON_CHAIN_PROPOSER_ADDRESS' .env | cut -d= -f2) && \
	ethrex_l2 config edit --on-chain-proposer $$OP $$C

# Variables

ethrex_PATH=$(shell pwd)/../..
ethrex_BIN_PATH=$(ethrex_PATH)/target/release/ethrex
ethrex_DEV_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/dev/docker-compose-dev.yaml
ethrex_L2_DOCKER_COMPOSE_PATH=./docker-compose-l2.yaml
ethrex_METRICS_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/metrics/docker-compose-metrics.yaml
ethrex_METRICS_OVERRIDES_L1_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/metrics/docker-compose-metrics-l1-dev.overrides.yaml
ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH=$(ethrex_PATH)/crates/blockchain/metrics/docker-compose-metrics-l2.overrides.yaml
CI_ETHREX_WORKDIR := /usr/local/bin

ethrex_L2_CONFIGS_PATH=$(shell pwd)/configs
ethrex_L2_SEQUENCER_CONFIG_FILE=sequencer_config.toml
ethrex_L2_PROVER_CLIENT_CONFIG_FILE=prover_client_config.toml
ethrex_L2_CONTRACTS_PATH=./contracts
L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

ethrex_L2_DEV_LIBMDBX=dev_ethrex_l2
ethrex_L1_DEV_LIBMDBX=dev_ethrex_l1
L1_PORT=8545
L2_PORT=1729
L1_AUTH_PORT=8551
# Used in the .env file. Ensure the same port is used for `ENGINE_API_RPC_URL`.
# https://github.com/lambdaclass/ethrex/issues/2358
# The authrpc.port has to be deprecated when using the l2 flag
L2_AUTH_PORT=8552

# Matches the ports used by the blockchain/metrics dir
L2_PROMETHEUS_METRICS_PORT = 3702

PROVER_ENV_FILE ?= ".env.prover" 

# Local L1
init-local-l1: ## 🚀 Initializes an L1 Lambda ethrex Client with Docker (Used with make init)
	docker compose -f ${ethrex_DEV_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L1_DOCKER_COMPOSE_PATH} up -d

init-l1: ## 🚀 Initializes an L1 Lambda ethrex Client
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features "dev" -- \
	--network ${L1_GENESIS_FILE_PATH} \
	--http.port ${L1_PORT} \
	--http.addr 0.0.0.0 \
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
    --http.addr 0.0.0.0 \
    --authrpc.port ${L1_AUTH_PORT} \
	--evm levm \
	--dev \
    --datadir ${ethrex_L1_DEV_LIBMDBX}

down-local-l1: ## 🛑 Shuts down the L1 Lambda ethrex Client
	docker compose -f ${ethrex_DEV_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L1_DOCKER_COMPOSE_PATH} down
	docker compose -f docker-compose-l2.yaml down

restart-local-l1: down-local-l1 rm-db-l1 init-local-l1 ## 🔄 Restarts the L1 Lambda ethrex Client

rm-db-l1: ## 🛑 Removes the DB used by the L1
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- removedb --datadir ${ethrex_L1_DEV_LIBMDBX}

# Contracts
clean-contract-deps: ## 🧹 Cleans the dependencies for the L1 contracts.
	rm -rf contracts/solc_out
	rm -rf contracts/lib

restart-contract-deps: clean-contract-deps ## 🔄 Restarts the dependencies for the L1 contracts.

deploy-l1: ## 📜 Deploys the L1 contracts
	DEPLOYER_CONTRACTS_PATH=contracts \
	CONFIGS_PATH=${ethrex_L2_CONFIGS_PATH} \
	SEQUENCER_CONFIG_FILE=${ethrex_L2_SEQUENCER_CONFIG_FILE} \
	cargo run --release --bin ethrex_l2_l1_deployer --manifest-path ${ethrex_L2_CONTRACTS_PATH}/Cargo.toml -- --deposit_rich

## Same as deploy-l1 but does not do deposits for rich accounts since that doesn't make sense for deployments to devnets/testnets i.e Sepolia
deploy-l1-testnet: ## 📜 Deploys the L1 contracts
	DEPLOYER_CONTRACTS_PATH=contracts \
	CONFIGS_PATH=${ethrex_L2_CONFIGS_PATH} \
	SEQUENCER_CONFIG_FILE=${ethrex_L2_SEQUENCER_CONFIG_FILE} \
	cargo run --release --bin ethrex_l2_l1_deployer --manifest-path ${ethrex_L2_CONTRACTS_PATH}/Cargo.toml

update-system-contracts:
	DEPLOYER_CONTRACTS_PATH=contracts \
	CONFIGS_PATH=${ethrex_L2_CONFIGS_PATH} \
	SEQUENCER_CONFIG_FILE=${ethrex_L2_SEQUENCER_CONFIG_FILE} \
	cargo run --release --bin ethrex_l2_system_contracts_updater --manifest-path ${ethrex_L2_CONTRACTS_PATH}/Cargo.toml -- ${L2_GENESIS_FILE_PATH}

# L2
init-l2: init-metrics init-l2-no-metrics ## 🚀 Initializes an L2 Lambda ethrex Client with metrics

init-l2-no-metrics: ## 🚀 Initializes an L2 Lambda ethrex Client
	if [ -z "$$BASED" ]; then \
		FEATURES="l2,metrics"; \
		echo "Running ethrex L2 vanilla"; \
	else \
		FEATURES="metrics,based"; \
		echo "Running ethrex L2 with based"; \
	fi; \
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features $$FEATURES -- \
	l2 init \
	--network ${L2_GENESIS_FILE_PATH} \
	--http.port ${L2_PORT} \
	--http.addr 0.0.0.0 \
	--authrpc.port ${L2_AUTH_PORT} \
	--metrics.port ${L2_PROMETHEUS_METRICS_PORT} \
	--datadir ${ethrex_L2_DEV_LIBMDBX}

init-metrics: ## 🚀 Initializes Grafana and Prometheus with containers
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} up -d

down-metrics: ## 🛑 Shuts down the metrics' containers
	docker compose -f ${ethrex_METRICS_DOCKER_COMPOSE_PATH} -f ${ethrex_METRICS_OVERRIDES_L2_DOCKER_COMPOSE_PATH} down

down-l2: ## 🛑 Shuts down the L2 Lambda ethrex Client
	pkill -f ethrex || exit 0

restart-l2: down-l2 rm-db-l2 init-l2 ## 🔄 Restarts the L2 Lambda ethrex Client

init-prover: ## 🚀 Initializes the Prover
	CONFIGS_PATH=${ethrex_L2_CONFIGS_PATH} \
	PROVER_CLIENT_CONFIG_FILE=${ethrex_L2_PROVER_CLIENT_CONFIG_FILE} \
	PROVER_ENV_FILE=${PROVER_ENV_FILE} \
	../../target/release/ethrex_prover

build-prover:
	@if [ -z "$$PROVER" ]; then \
		echo "Error: ProverType (PROVER) is missing. Running in exec mode."; \
		echo "Please provide it as an argument:"; \
		echo "make init-prover PROVER=<prover_type: (risc0, sp1, pico)> <G=true>"; \
		echo "The prover can also be run with GPU (G)"; \
	fi; \

	@if [ -z "$$G" ]; then \
		GPU=""; \
	else \
		GPU=",gpu"; \
	fi; \

	CONFIGS_PATH=${ethrex_L2_CONFIGS_PATH} \
	PROVER_CLIENT_CONFIG_FILE=${ethrex_L2_PROVER_CLIENT_CONFIG_FILE} \
	PROVER_ENV_FILE=${PROVER_ENV_FILE} \
	RUSTFLAGS='-C target-cpu=native' \
	cargo build --release --features "$$PROVER$$GPU,l2" \
	--manifest-path ./prover/Cargo.toml  \
	--bin ethrex_prover

rm-db-l2: ## 🛑 Removes the DB used by the L2
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features l2 -- l2 removedb --datadir ${ethrex_L2_DEV_LIBMDBX}

integration-test: build-prover rm-db-l2 rm-db-l1
	# We create an empty .env file simply because if the file
	# does not exists, the container fails to write to it.
	touch .env
	CI_ETHREX_WORKDIR=${CI_ETHREX_WORKDIR} docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} down
	CI_ETHREX_WORKDIR=${CI_ETHREX_WORKDIR} docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} up --detach
	RUST_LOG=info,ethrex_prover_lib=debug make init-prover & cargo test l2 --release -- --nocapture --test-threads=1
	killall ethrex_prover

# State reconstruction tests
state-diff-test: init-l2
	rm -rf $$PWD/store
	cargo run --release --manifest-path ../../cmd/ethrex_l2/Cargo.toml --bin ethrex_l2 -- config create ci --default
	cargo run --release --manifest-path ../../cmd/ethrex_l2/Cargo.toml --bin ethrex_l2 -- config set ci
	cargo run --release --manifest-path ../../cmd/ethrex_l2/Cargo.toml --bin ethrex_l2 -- stack reconstruct -g ../../test_data/genesis-l2.json -b ../../test_data/blobs/ -s $$PWD/store -c 0x0007a881CD95B1484fca47615B64803dad620C8d
	cargo b --manifest-path ../../Cargo.toml --release
	make restart ethrex_L2_DEV_LIBMDBX=$$PWD/store 2>&1 >/dev/null &
	cargo test state_reconstruct --release

# Purge L2's state
UNAME_S:=$(shell uname -s)
# This directory is set by crates/l2/utils/prover/save_state.rs -> const DEFAULT_DATADIR
PROJECT_NAME:=ethrex_l2_state

ifeq ($(UNAME_S),Linux)
    PROJECT_PATH := $(HOME)/.local/share/${PROJECT_NAME}
else ifeq ($(UNAME_S),Darwin)
    PROJECT_PATH := $(HOME)/Library/Application\ Support/${PROJECT_NAME}
else
    $(error Unsupported platform: $(UNAME_S))
endif

purge_prover_state: ## 🧹 Removes the L2 state, only use to start fresh.
	@echo "Are you sure you want to delete the directory: $(PROJECT_PATH) ? [y/n]"
	@read answer; \
	if [ "$$answer" != "y" ]; then \
		echo "Operation canceled."; \
	fi; \
	rm -rf $(PROJECT_PATH); \
	echo "Directory deleted."

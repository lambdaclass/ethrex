.PHONY: help init down clean restart cli update-cli-contracts init-local-l1 init-l1 down-local-l1 restart-local-l1 rm-db-l1 clean-contract-deps restart-contract-deps deploy-l1 init-l2 down-l2 restart-l2 init-prover rm-db-l2 purge_prover_state ci_test test

.DEFAULT_GOAL := help

L2_GENESIS_FILE_PATH=../../test_data/genesis-l2.json
L1_GENESIS_FILE_PATH=../../test_data/genesis-l1.json

# Basic
help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

init: init-local-l1 deploy-l1 update-system-contracts  init-l2 ## 🚀 Initializes a localnet with Lambda ethrex client as both L1 and L2

down: down-local-l1 down-l2 down-metrics## 🛑 Shuts down the localnet

clean: clean-contract-deps ## 🧹 Cleans the localnet

restart: restart-local-l1 deploy-l1 restart-l2 ## 🔄 Restarts the localnet

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


ethrex_L2_CONTRACTS_PATH=./contracts
L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

ethrex_L2_DEV_LIBMDBX=dev_ethrex_l2
ethrex_L1_DEV_LIBMDBX=dev_ethrex_l1
L1_PORT=8545
L2_PORT=1729
L1_AUTH_PORT=8551
# Used in the .env file. Ensure the same port is used for `ENGINE_API_RPC_URL`.
L2_AUTH_PORT=8552

# Matches the ports used by the blockchain/metrics dir
L2_PROMETHEUS_METRICS_PORT = 3702

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

restart-local-l1: down-local-l1 init-local-l1 ## 🔄 Restarts the L1 Lambda ethrex Client

rm-db-l1: ## 🛑 Removes the DB used by the L1
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- removedb --datadir ${ethrex_L1_DEV_LIBMDBX}

# Contracts
clean-contract-deps: ## 🧹 Cleans the dependencies for the L1 contracts.
	rm -rf contracts/solc_out
	rm -rf contracts/lib

restart-contract-deps: clean-contract-deps ## 🔄 Restarts the dependencies for the L1 contracts.

deploy-l1: ## 📜 Deploys the L1 contracts
	DEPLOYER_CONTRACTS_PATH=contracts cargo run --release --bin ethrex_l2_l1_deployer --manifest-path ${ethrex_L2_CONTRACTS_PATH}/Cargo.toml

update-system-contracts:
	DEPLOYER_CONTRACTS_PATH=contracts cargo run --release --bin ethrex_l2_system_contracts_updater --manifest-path ${ethrex_L2_CONTRACTS_PATH}/Cargo.toml -- ${L2_GENESIS_FILE_PATH}

# L2
init-l2: init-metrics ## 🚀 Initializes an L2 Lambda ethrex Client
	if [[ -z "$$BASED" ]]; then \
		FEATURES="l2,metrics"; \
		echo "Running ethrex L2 vanilla"; \
	else \
		FEATURES="metrics,based"; \
		echo "Running ethrex L2 with based"; \
	fi; \
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features $$FEATURES -- \
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

restart-l2: down-l2 init-l2 ## 🔄 Restarts the L2 Lambda ethrex Client

init-prover: ## 🚀 Initializes the Prover
	@if [ -z "$$T" ]; then \
		echo "Error: ProverType (T) is missing."; \
		echo "Please provide it as an argument:"; \
		echo "make init-prover T=<prover_type: (risc0, sp1)> <G=true>."; \
		echo "The prover can also be run with GPU (G)"; \
		exit 1; \
	fi; \

	@if [ -z "$$G" ]; then \
		GPU=""; \
	else \
		GPU=",gpu"; \
	fi; \
	cargo run --release --features "build_$$T$$GPU" --manifest-path ./prover/Cargo.toml --bin ethrex_prover -- $$T

rm-db-l2: ## 🛑 Removes the DB used by the L2
	cargo run --release --manifest-path ../../Cargo.toml --bin ethrex -- removedb --datadir ${ethrex_L2_DEV_LIBMDBX}

# Testing
# cargo test l2 will match tests that has l2 in the function.
# Tests are at `crates/l2/tests/tests.rs`
# https://doc.rust-lang.org/book/ch11-02-running-tests.html#filtering-to-run-multiple-tests
ci_test: ## 🚧 Runs the L2's integration test, used by the github's CI
	touch .env
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} down
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} up -d --build
	cargo test l2 --release -- --nocapture --test-threads=1
	docker compose -f ${ethrex_L2_DOCKER_COMPOSE_PATH} down

test: ## 🚧 Runs the L2's integration test, run `make init` and in a new terminal make test
	BRIDGE_ADDRESS=$$(grep 'L1_WATCHER_BRIDGE_ADDRESS' .env | cut -d= -f2) ON_CHAIN_PROPOSER_ADDRESS=$$(grep 'COMMITTER_ON_CHAIN_PROPOSER_ADDRESS' .env | cut -d= -f2) cargo test l2 --release -- --nocapture --test-threads=1


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

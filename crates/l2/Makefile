# ##############################################################################
#
# L2 Makefile
#
# This Makefile provides a streamlined interface for managing the L2 development
# environment. It supports running services via Docker Compose or directly on
# the host machine.
#
# ##############################################################################

.DEFAULT_GOAL := help

# ==============================================================================
# Configuration
# ==============================================================================

# If a .env file exists, include it. This allows for easy local configuration.
# For example, you can set L1_RPC_URL or other variables in a .env file.
-include .env

# Export all variables prefixed with ETHREX_ so they are available to sub-processes.
export $(shell sed -n 's/^\(ETHREX_[^=]*\)=.*/\1/p' .env)


# Path variables
ETHREX_PATH ?= $(shell pwd)/../..
ETHREX_BIN_PATH ?= $(ETHREX_PATH)/target/release/ethrex
L1_DOCKER_COMPOSE ?= $(ETHREX_PATH)/crates/blockchain/dev/docker-compose-dev.yaml
L2_DOCKER_COMPOSE ?= ./docker-compose-l2.yaml
PROVER_DOCKER_COMPOSE ?= ./docker-compose-prover.yaml # Assumed, to be created later

# L1 Configuration
L1_GENESIS_FILE_PATH ?= ../../test_data/genesis-l1-dev.json
ETHREX_L1_DEV_LIBMDBX ?= dev_ethrex_l1
L1_PORT ?= 8545
L1_AUTH_PORT ?= 8551
L1_RPC_ADDRESS ?= 0.0.0.0

# L2 Configuration
L2_GENESIS_FILE_PATH ?= ../../test_data/genesis-l2.json
ETHREX_L2_DEV_LIBMDBX ?= dev_ethrex_l2
L2_PORT ?= 1729
L2_RPC_ADDRESS ?= 0.0.0.0
L1_RPC_URL ?= http://localhost:8545
PROOF_COORDINATOR_ADDRESS ?= 127.0.0.1
L2_PROMETHEUS_METRICS_PORT ?= 3702

# Prover Configuration
PROVER_HTTP_ADDR ?= 127.0.0.1
PROVER_HTTP_PORT ?= 3900
PROVER_LOG_LEVEL ?= debug


# ==============================================================================
# Top-Level Targets
# ==============================================================================

.PHONY: help up down clean build

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

up: up-l1-docker up-l2-docker ## 🚀 Start L1 and L2 services using Docker Compose.

down: down-l1-docker down-l2-docker ## 🛑 Stop all Docker Compose services.

clean: ## 🧹 Clean up project files.
	@echo "Cleaning up..."
	@echo "Clean complete."

build: build-ethrex build-prover ## 🏗️ Build ethrex and prover binaries.

build-ethrex:
	@echo "Building ethrex binary..."
	@cargo build --release --manifest-path $(ETHREX_PATH)/Cargo.toml --bin ethrex --features "l2,rollup_storage_libmdbx,metrics,dev"

build-prover:
	@echo "Building prover binary..."
	@if [ -z "$(PROVER)" ]; then \
		echo "Error: ProverType (PROVER) is missing. Running in exec mode."; \
		echo "Please provide it as an argument:"; \
		echo "make build-prover PROVER=<prover_type: (risc0, sp1)> <G=true>"; \
		echo "The prover can also be run with GPU (G)"; \
		exit 1; \
	fi;
	@if [ -z "$$G" ]; then \
		GPU=""; \
	else \
		GPU=",gpu"; \
	fi;
	RUSTFLAGS='-C target-cpu=native' \
	cargo build --release --features "$$PROVER$$GPU,l2" \
	--manifest-path ./prover/Cargo.toml  \
	--bin ethrex_prover


# ==============================================================================
# Docker Compose Targets
# ==============================================================================

.PHONY: up-l1-docker down-l1-docker up-l2-docker down-l2-docker up-prover-docker down-prover-docker

up-l1-docker: ## 🐳 Start L1 in Docker.
	@echo "Starting L1 Docker container..."
	@docker compose -f $(L1_DOCKER_COMPOSE) up -d

down-l1-docker: ## 🐳 Stop L1 in Docker.
	@echo "Stopping L1 Docker container..."
	@docker compose -f $(L1_DOCKER_COMPOSE) down

up-l2-docker: ## 🐳 Start L2 in Docker.
	@echo "Starting L2 Docker container..."
	@docker compose -f $(L2_DOCKER_COMPOSE) up -d

down-l2-docker: ## 🐳 Stop L2 in Docker.
	@echo "Stopping L2 Docker container..."
	@docker compose -f $(L2_DOCKER_COMPOSE) down

up-prover-docker: ## 🐳 Start Prover in Docker.
	@echo "Starting Prover Docker container..."
	@docker compose -f $(PROVER_DOCKER_COMPOSE) up -d

down-prover-docker: ## 🐳 Stop Prover in Docker.
	@echo "Stopping Prover Docker container..."
	@docker compose -f $(PROVER_DOCKER_COMPOSE) down


# ==============================================================================
# Host Process Targets
# ==============================================================================

.PHONY: run-l1-host run-l2-host run-prover-host

run-l1-host: ## 🏃 Run L1 node on the host.
	@echo "Starting L1 on host..."
	@cargo run --release --manifest-path $(ETHREX_PATH)/Cargo.toml --bin ethrex --features "dev" -- \
		--network $(L1_GENESIS_FILE_PATH) \
		--http.port $(L1_PORT) \
		--http.addr $(L1_RPC_ADDRESS) \
		--authrpc.port $(L1_AUTH_PORT) \
		--dev \
		--datadir $(ETHREX_L1_DEV_LIBMDBX)

run-l2-host: ## 🏃 Run L2 node on the host.
	@echo "Starting L2 on host..."
	@cargo run --release --manifest-path $(ETHREX_PATH)/Cargo.toml --bin ethrex --features "l2,rollup_storage_libmdbx,metrics" -- \
	l2 init \
	--watcher.block-delay 0 \
	--network $(L2_GENESIS_FILE_PATH) \
	--http.port $(L2_PORT) \
	--http.addr $(L2_RPC_ADDRESS) \
	--metrics \
	--metrics.port $(L2_PROMETHEUS_METRICS_PORT) \
	--evm levm \
	--datadir $(ETHREX_L2_DEV_LIBMDBX) \
	--l1.bridge-address $$ETHREX_WATCHER_BRIDGE_ADDRESS \
	--l1.on-chain-proposer-address $$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS \
	--eth.rpc-url $(L1_RPC_URL) \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--committer.l1-private-key $$ETHREX_COMMITTER_L1_PRIVATE_KEY \
	--proof-coordinator.l1-private-key $$ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY \
	--proof-coordinator.addr $(PROOF_COORDINATOR_ADDRESS)

run-prover-host: ## 🏃 Run Prover on the host.
	@echo "Starting Prover on host..."
	@$(ETHREX_PATH)/target/release/ethrex_prover \
	--http.addr $(PROVER_HTTP_ADDR) \
  	--http.port $(PROVER_HTTP_PORT) \
	--log.level $(PROVER_LOG_LEVEL)

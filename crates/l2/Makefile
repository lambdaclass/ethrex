.DEFAULT_GOAL := help

.PHONY: help init down clean init-local-l1 down-local-l1 clean-local-l1 init-l2 down-l2 deploy-l1 deploy-block-executor deploy-inbox setup-prover

L2_GENESIS_FILE_PATH=../../test_data/genesis-l2.json

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

init: init-local-l1 deploy-l1 init-l2 ## 🚀 Initializes a localnet with Lambda Ethereum Rust client as both L1 and L2

down: down-local-l1 down-l2 ## 🛑 Shuts down the localnet

clean: clean-contract-deps ## 🧹 Cleans the localnet

restart: restart-local-l1 deploy-l1 restart-l2 ## 🔄 Restarts the localnet

cli: ## 🛠️ Installs the L2 Lambda Ethereum Rust CLI
	cargo install --path ${ETHEREUM_RUST_PATH}/cmd/ethereum_rust_l2/ --force

# Variables

ETHEREUM_RUST_PATH=$(shell pwd)/../..
ETHEREUM_RUST_BIN_PATH=$(ETHEREUM_RUST_PATH)/target/release/ethereum_rust
ETHEREUM_RUST_DEV_DOCKER_COMPOSE_PATH=$(ETHEREUM_RUST_PATH)/crates/blockchain/dev/docker-compose-dev.yaml

ETHEREUM_RUST_L2_CONTRACTS_PATH=./contracts
L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

# Local L1

init-local-l1: ## 🚀 Initializes an L1 Lambda Ethereum Rust Client
	docker compose -f ${ETHEREUM_RUST_DEV_DOCKER_COMPOSE_PATH} up -d

down-local-l1: ## 🛑 Shuts down the L1 Lambda Ethereum Rust Client
	docker compose -f ${ETHEREUM_RUST_DEV_DOCKER_COMPOSE_PATH} down

restart-local-l1: down-local-l1 init-local-l1 ## 🔄 Restarts the L1 Lambda Ethereum Rust Client

# Contracts

clean-contract-deps: ## 🧹 Cleans the dependencies for the L1 contracts.
	rm -rf contracts/solc_out
	rm -rf contracts/lib

restart-contract-deps: clean-contract-deps ## 🔄 Restarts the dependencies for the L1 contracts.

deploy-l1: ## 📜 Deploys the L1 contracts
	cargo run --release --bin ethereum_rust_l2_l1_deployer --manifest-path ${ETHEREUM_RUST_L2_CONTRACTS_PATH}/Cargo.toml

# L2

init-l2: ## 🚀 Initializes an L2 Lambda Ethereum Rust Client
	cargo run --release --manifest-path ../../Cargo.toml --bin ethereum_rust --features l2 -- --network ${L2_GENESIS_FILE_PATH} --http.port 1729 

down-l2: ## 🛑 Shuts down the L2 Lambda Ethereum Rust Client
	pkill -f ethereum_rust || exit 0

restart-l2: down-l2 init-l2 ## 🔄 Restarts the L2 Lambda Ethereum Rust Client

init-l2-prover: ## 🚀 Initializes the Prover
	cargo run --release --features build_zkvm --manifest-path ../../Cargo.toml --bin ethereum_rust_prover

init-l2-prover-gpu: ## 🚀 Initializes the Prover with GPU support
	cargo run --release --features "build_zkvm,cuda" --manifest-path ../../Cargo.toml --bin ethereum_rust_prover

.PHONY: init down clean init-local-l1 down-local-l1 clean-local-l1 init-l2 down-l2 deploy-l1 deploy-block-executor deploy-inbox setup-prover

L2_GENESIS_FILE_PATH=../../test_data/genesis-l2.json

init: init-local-l1 contract-deps setup-prover deploy-l1 init-l2

down: down-local-l1 down-l2

clean: clean-local-l1 clean-contract-deps

restart: restart-local-l1 restart-contract-deps restart-l2

# Contracts

FOUNDRY_PROJECT_HOME=$(shell pwd)/contracts
L1_RPC_URL=http://localhost:8545
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

contract-deps:
	mkdir -p ${FOUNDRY_PROJECT_HOME}
	forge install foundry-rs/forge-std --no-git --root ${FOUNDRY_PROJECT_HOME}
	forge install succinctlabs/sp1-contracts --no-git --root ${FOUNDRY_PROJECT_HOME}

clean-contract-deps:
	rm -rf contracts/lib

restart-contract-deps: clean-contract-deps contract-deps

deploy-l1:
	cd ${FOUNDRY_PROJECT_HOME} && \
	forge script script/DeployL1.s.sol:DeployL1Script --rpc-url ${L1_RPC_URL} --private-key ${L1_PRIVATE_KEY} --broadcast

deploy-block-executor:
	forge create ${FOUNDRY_PROJECT_HOME}/src/l1/OnChainOperator.sol:OnChainOperator --rpc-url ${L1_RPC_URL} --private-key ${L1_PRIVATE_KEY}

deploy-inbox:
	forge create ${FOUNDRY_PROJECT_HOME}/src/l1/Inbox.sol:Inbox --rpc-url ${L1_RPC_URL} --private-key ${L1_PRIVATE_KEY}

# Local L1

init-local-l1:
	mkdir -p volumes/ volumes/reth volumes/reth/data
	docker compose -f docker-compose-l2.yml up -d

down-local-l1:
	docker compose -f docker-compose-l2.yml down

clean-local-l1:
	rm -rf volumes/

restart-local-l1: down-local-l1 clean-local-l1 init-local-l1

# L2

init-l2:
	cargo run --release --manifest-path ../../Cargo.toml --bin ethereum_rust --features l2 -- --network ${L2_GENESIS_FILE_PATH} --http.port 1729 

down-l2:
	pkill -f ethereum_rust || exit 0

restart-l2: down-l2 init-l2

# Prover

SP1_PROGRAM_PATH=$(shell pwd)/prover/sp1/program

setup-prover:
	cd ${SP1_PROGRAM_PATH} && \
	cargo prove build

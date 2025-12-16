.PHONY: build lint test clean run-image build-image clean-vectors \
		setup-hive test-pattern-default run-hive run-hive-debug clean-hive-logs \
		load-test-fibonacci load-test-io run-hive-eels-blobs

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

build: ## 🔨 Build the client
	cargo build --workspace

lint-l1:
	cargo clippy --lib --bins -F debug,sync-test \
		--release -- -D warnings

lint-l2:
	cargo clippy --all-targets -F debug,sync-test,l2,l2-sql \
		--workspace --exclude ethrex-prover --exclude guest_program \
		--release -- -D warnings

lint-gpu:
	cargo clippy --all-targets -F debug,sync-test,l2,l2-sql,,sp1,risc0,gpu \
		--workspace --exclude ethrex-prover --exclude guest_program \
		--release -- -D warnings

lint: lint-l1 lint-l2 ## 🧹 Linter check

CRATE ?= *
# CAUTION: It is important that the ethrex-l2 crate remains excluded here,
# as its tests depend on external setup that is not handled by this Makefile.
test: ## 🧪 Run each crate's tests
	cargo test -p '$(CRATE)' --workspace --exclude ethrex-l2

clean: clean-vectors ## 🧹 Remove build artifacts
	cargo clean
	rm -rf hive

STAMP_FILE := .docker_build_stamp
$(STAMP_FILE): $(shell find crates cmd -type f -name '*.rs') Cargo.toml Dockerfile
	docker build -t ethrex:local .
	touch $(STAMP_FILE)

build-image: $(STAMP_FILE) ## 🐳 Build the Docker image

run-image: build-image ## 🏃 Run the Docker image
	docker run --rm -p 127.0.0.1:8545:8545 ethrex:main --http.addr 0.0.0.0

dev: ## 🏃 Run the ethrex client in DEV_MODE with the InMemory Engine
	cargo run --bin ethrex -- \
			--network ./fixtures/genesis/l1.json \
			--http.port 8545 \
			--http.addr 0.0.0.0 \
			--authrpc.port 8551 \
			--dev \
			--datadir memory

ETHEREUM_PACKAGE_REVISION := 82e5a7178138d892c0c31c3839c89d53ffd42d9a
ETHEREUM_PACKAGE_DIR := ethereum-package

checkout-ethereum-package: ## 📦 Checkout specific Ethereum package revision
	@if [ ! -d "$(ETHEREUM_PACKAGE_DIR)" ]; then \
		echo "Cloning ethereum-package repository..."; \
		git clone --quiet https://github.com/ethpandaops/ethereum-package $(ETHEREUM_PACKAGE_DIR); \
	fi
	@cd $(ETHEREUM_PACKAGE_DIR) && \
	CURRENT_REV=$$(git rev-parse HEAD) && \
	if [ "$$CURRENT_REV" != "$(ETHEREUM_PACKAGE_REVISION)" ]; then \
		echo "Current HEAD ($$CURRENT_REV) is not the target revision. Checking out $(ETHEREUM_PACKAGE_REVISION)..."; \
		git fetch --quiet && \
		git checkout --quiet $(ETHEREUM_PACKAGE_REVISION); \
	else \
		echo "ethereum-package is already at the correct revision."; \
	fi

ENCLAVE ?= lambdanet
KURTOSIS_CONFIG_FILE ?= ./fixtures/networks/default.yaml

# If on a Mac, use OrbStack to run Docker containers because Docker Desktop doesn't work well with Kurtosis
localnet: build-image checkout-ethereum-package ## 🌐 Start kurtosis network
	@set -e; \
	trap 'printf "\nStopping localnet...\n"; $(MAKE) stop-localnet || true; exit 0' INT TERM HUP QUIT; \
	cp metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json ethereum-package/src/grafana/ethrex_l1_perf.json; \
	kurtosis run --enclave $(ENCLAVE) ethereum-package --args-file $(KURTOSIS_CONFIG_FILE); \
	CID=$$(docker ps -q --filter ancestor=ethrex:local | head -n1); \
	if [ -n "$$CID" ]; then docker logs -f $$CID || true; else echo "No ethrex container found; skipping logs."; fi

stop-localnet: ## 🛑 Stop local network
	kurtosis enclave stop $(ENCLAVE)
	kurtosis enclave rm $(ENCLAVE) --force

HIVE_BRANCH ?= master

setup-hive: ## 🐝 Set up Hive testing framework
	if [ -d "hive" ]; then \
		cd hive && \
		git fetch origin && \
		git checkout $(HIVE_BRANCH) && \
		git pull origin $(HIVE_BRANCH) && \
		go build .; \
	else \
		git clone --branch $(HIVE_BRANCH) https://github.com/ethereum/hive && \
		cd hive && \
		git checkout $(HIVE_BRANCH) && \
		go build .; \
	fi

TEST_PATTERN ?= /
SIM_LOG_LEVEL ?= 3
SIM_PARALLELISM ?= 16
# https://github.com/ethereum/execution-apis/pull/627 changed the simulation to use a pre-merge genesis block, so we need to pin to a commit before that
ifeq ( $(SIMULATION) , ethereum/rpc-compat )
SIM_BUILDARG_FLAG = --sim.buildarg "branch=d08382ae5c808680e976fce4b73f4ba91647199b"
endif

# Runs a Hive testing suite. A web interface showing the results is available at http://127.0.0.1:8080 via the `view-hive` target.
# The endpoints tested can be filtered by supplying a test pattern in the form "/endpoint_1|endpoint_2|..|endpoint_n".
# For example, to run the rpc-compat suites for eth_chainId & eth_blockNumber, you should run:
# `make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_blockNumber"`
# The simulation log level can be set using SIM_LOG_LEVEL (from 1 up to 4).

HIVE_CLIENT_FILE := ../fixtures/hive/clients.yaml

run-hive: build-image setup-hive ## 🧪 Run Hive testing suite
	- cd hive && ./hive --client-file $(HIVE_CLIENT_FILE) --client ethrex --sim $(SIMULATION) --sim.limit "$(TEST_PATTERN)" --sim.parallelism $(SIM_PARALLELISM) --sim.loglevel $(SIM_LOG_LEVEL) $(SIM_BUILDARG_FLAG)
	$(MAKE) view-hive

run-hive-all: build-image setup-hive ## 🧪 Run all Hive testing suites
	- cd hive && ./hive --client-file $(HIVE_CLIENT_FILE) --client ethrex --sim ".*" --sim.parallelism $(SIM_PARALLELISM) --sim.loglevel $(SIM_LOG_LEVEL)
	$(MAKE) view-hive

run-hive-debug: build-image setup-hive ## 🐞 Run Hive testing suite in debug mode
	cd hive && ./hive --sim $(SIMULATION) --client-file $(HIVE_CLIENT_FILE)  --client ethrex --sim.loglevel 4 --sim.limit "$(TEST_PATTERN)" --sim.parallelism "$(SIM_PARALLELISM)" --docker.output $(SIM_BUILDARG_FLAG)

# EELS Hive
TEST_PATTERN_EELS ?= .*fork_Paris.*|.*fork_Shanghai.*|.*fork_Cancun.*|.*fork_Prague.*
run-hive-eels: build-image setup-hive ## 🧪 Generic command for running Hive EELS tests. Specify EELS_SIM
	- cd hive && ./hive --client-file $(HIVE_CLIENT_FILE) --client ethrex --sim $(EELS_SIM) --sim.limit "$(TEST_PATTERN_EELS)" --sim.parallelism $(SIM_PARALLELISM) --sim.loglevel $(SIM_LOG_LEVEL) --sim.buildarg fixtures=$(shell cat tooling/ef_tests/blockchain/.fixtures_url)

run-hive-eels-engine: ## Run hive EELS Engine tests
	$(MAKE) run-hive-eels EELS_SIM=ethereum/eels/consume-engine

run-hive-eels-rlp: ## Run hive EELS RLP tests
	$(MAKE) run-hive-eels EELS_SIM=ethereum/eels/consume-rlp

run-hive-eels-blobs: ## Run hive EELS Blobs tests
	$(MAKE) run-hive-eels EELS_SIM=ethereum/eels/execute-blobs

clean-hive-logs: ## 🧹 Clean Hive logs
	rm -rf ./hive/workspace/logs

view-hive: ## 🛠️ Builds hiveview with the logs from the hive execution
	cd hive && go build ./cmd/hiveview && ./hiveview --serve --logdir ./workspace/logs

start-node-with-flamegraph: rm-test-db ## 🚀🔥 Starts an ethrex client used for testing
	echo "Running the test-node with LEVM"; \

	sudo -E CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--bin ethrex \
	-- \
	--network fixtures/genesis/l2.json \
	--http.port 1729 \
	--dev \
	--datadir test_ethrex

load-test: ## 🚧 Runs a load-test. Run make start-node-with-flamegraph and in a new terminal make load-node
	cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t eth-transfers

load-test-erc20:
	cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t erc20

load-test-fibonacci:
	cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t fibonacci

load-test-io:
	cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t io-heavy

rm-test-db:  ## 🛑 Removes the DB used by the ethrex client used for testing
	sudo cargo run --release --bin ethrex -- removedb --force --datadir test_ethrex

fixtures/ERC20/ERC20.bin: ## 🔨 Build the ERC20 contract for the load test
	solc ./fixtures/contracts/ERC20/ERC20.sol -o $@

sort-genesis-files:
	cd ./tooling/genesis && cargo run

# Using & so make calls this recipe only once per run
mermaid-init.js mermaid.min.js &:
	@# Required for mdbook-mermaid to work
	@mdbook-mermaid install . \
		|| (echo "mdbook-mermaid invocation failed, remember to install docs dependencies first with \`make docs-deps\`" \
		&& exit 1)

docs-deps: ## 📦 Install dependencies for generating the documentation
	cargo install --version 0.9.4 mdbook-katex
	cargo install --version 0.7.7 mdbook-linkcheck
	cargo install --version 0.8.0 mdbook-alerts
	cargo install --version 0.15.0 mdbook-mermaid

docs: mermaid-init.js mermaid.min.js ## 📚 Generate the documentation
	mdbook build

docs-serve: mermaid-init.js mermaid.min.js ## 📚 Generate and serve the documentation
	mdbook serve --open

update-cargo-lock: ## 📦 Update Cargo.lock files
	cargo tree
	cargo tree --manifest-path crates/l2/prover/src/guest_program/src/sp1/Cargo.toml
	cargo tree --manifest-path crates/l2/prover/src/guest_program/src/risc0/Cargo.toml
	cargo tree --manifest-path crates/l2/prover/src/guest_program/src/zisk/Cargo.toml
	cargo tree --manifest-path crates/l2/prover/src/guest_program/src/openvm/Cargo.toml
	cargo tree --manifest-path crates/l2/tee/quote-gen/Cargo.toml

.PHONY: build lint test clean run-image build-image clean-vectors \
		setup-hive test-pattern-default run-hive run-hive-debug clean-hive-logs \
		load-test-fibonacci load-test-io

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

build: ## 🔨 Build the client
	cargo build --workspace

lint: ## 🧹 Linter check
	# Note that we are compiling without the "gpu" feature (see #4048 for why)
	# To compile with it you can replace '-F' with '--all-features', but you need to have nvcc installed
	cargo clippy --all-targets -F debug,risc0,sp1,sync-test \
		--workspace --exclude ethrex-replay --exclude ethrex-prover --exclude zkvm_interface --exclude ef_tests-blockchain \
		--release -- -D warnings

CRATE ?= *
test: ## 🧪 Run each crate's tests
	cargo test -p '$(CRATE)' --workspace --exclude ethrex-levm --exclude ef_tests-blockchain --exclude ef_tests-state --exclude ethrex-l2 -- --skip test_contract_compilation

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
			--evm levm \
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
LOCALNET_CONFIG_FILE ?= ./fixtures/networks/network_params.yaml

# If on a Mac, use OrbStack to run Docker containers because Docker Desktop doesn't work well with Kurtosis
localnet: stop-localnet-silent build-image checkout-ethereum-package ## 🌐 Start local network
	cp metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json ethereum-package/src/grafana/ethrex_l1_perf.json
	kurtosis run --enclave $(ENCLAVE) ethereum-package --args-file $(LOCALNET_CONFIG_FILE)
	docker logs -f $$(docker ps -q --filter ancestor=ethrex:local)

hoodi: stop-localnet-silent build-image checkout-ethereum-package ## 🌐 Start client in hoodi network
	cp metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json ethereum-package/src/grafana/ethrex_l1_perf.json
	kurtosis run --enclave $(ENCLAVE) ethereum-package --args-file fixtures/network/hoodi.yaml
	docker logs -f $$(docker ps -q --filter ancestor=ethrex:local)

stop-localnet: ## 🛑 Stop local network
	kurtosis enclave stop $(ENCLAVE)
	kurtosis enclave rm $(ENCLAVE) --force

stop-localnet-silent:
	@echo "Double checking local net is not already started..."
	@kurtosis enclave stop $(ENCLAVE) >/dev/null 2>&1 || true
	@kurtosis enclave rm $(ENCLAVE) --force >/dev/null 2>&1 || true

HIVE_BRANCH ?= master

setup-hive: ## 🐝 Set up Hive testing framework
	if [ -d "hive" ]; then \
		cd hive && \
		git fetch origin && \
		git checkout $(HIVE_BRANCH) && \
		git pull origin $(HIVE_BRANCH) && \
		go build .; \
	else \
		git clone --branch $(HIVE_BRANCH) https://github.com/lambdaclass/hive && \
		cd hive && \
		git checkout $(HIVE_BRANCH) && \
		go build .; \
	fi

TEST_PATTERN ?= /
SIM_LOG_LEVEL ?= 3
SIM_PARALLELISM ?= 16

# Runs a Hive testing suite. A web interface showing the results is available at http://127.0.0.1:8080 via the `view-hive` target.
# The endpoints tested can be filtered by supplying a test pattern in the form "/endpoint_1|endpoint_2|..|endpoint_n".
# For example, to run the rpc-compat suites for eth_chainId & eth_blockNumber, you should run:
# `make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_blockNumber"`
# The simulation log level can be set using SIM_LOG_LEVEL (from 1 up to 4).

HIVE_CLIENT_FILE := ../fixtures/hive/clients.yaml

run-hive: build-image setup-hive ## 🧪 Run Hive testing suite
	- cd hive && ./hive --client-file $(HIVE_CLIENT_FILE) --client ethrex --sim $(SIMULATION) --sim.limit "$(TEST_PATTERN)" --sim.parallelism $(SIM_PARALLELISM) --sim.loglevel $(SIM_LOG_LEVEL)
	$(MAKE) view-hive

run-hive-all: build-image setup-hive ## 🧪 Run all Hive testing suites
	- cd hive && ./hive --client-file $(HIVE_CLIENT_FILE) --client ethrex --sim ".*" --sim.parallelism $(SIM_PARALLELISM) --sim.loglevel $(SIM_LOG_LEVEL)
	$(MAKE) view-hive

run-hive-debug: build-image setup-hive ## 🐞 Run Hive testing suite in debug mode
	cd hive && ./hive --sim $(SIMULATION) --client-file $(HIVE_CLIENT_FILE)  --client ethrex --sim.loglevel 4 --sim.limit "$(TEST_PATTERN)" --sim.parallelism "$(SIM_PARALLELISM)" --docker.output

# EEST Hive
TEST_PATTERN_EEST ?= .*fork_Paris.*|.*fork_Shanghai.*|.*fork_Cancun.*|.*fork_Prague.*
run-hive-eest: build-image setup-hive ## 🧪 Generic command for running Hive EEST tests. Specify EEST_SIM
	- cd hive && ./hive --client-file $(HIVE_CLIENT_FILE) --client ethrex --sim $(EEST_SIM) --sim.limit "$(TEST_PATTERN_EEST)" --sim.parallelism $(SIM_PARALLELISM) --sim.loglevel $(SIM_LOG_LEVEL) --sim.buildarg fixtures=$(shell cat tooling/ef_tests/blockchain/.fixtures_url)

run-hive-eest-engine: ## Run hive EEST Engine tests
	$(MAKE) run-hive-eest EEST_SIM=ethereum/eest/consume-engine

run-hive-eest-rlp: ## Run hive EEST Engine tests
	$(MAKE) run-hive-eest EEST_SIM=ethereum/eest/consume-rlp

clean-hive-logs: ## 🧹 Clean Hive logs
	rm -rf ./hive/workspace/logs

view-hive: ## 🛠️ Builds hiveview with the logs from the hive execution
	cd hive && go build ./cmd/hiveview && ./hiveview --serve --logdir ./workspace/logs

start-node-with-flamegraph: rm-test-db ## 🚀🔥 Starts an ethrex client used for testing
	@if [ -z "$$L" ]; then \
		LEVM="revm"; \
		echo "Running the test-node without the LEVM feature"; \
		echo "If you want to use levm, run the target with an L at the end: make <target> L=1"; \
	else \
		LEVM="levm"; \
		echo "Running the test-node with the LEVM feature"; \
	fi; \
	sudo -E CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--bin ethrex \
	-- \
	--evm $$LEVM \
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
	cargo install mdbook mdbook-alerts mdbook-mermaid mdbook-linkcheck mdbook-katex

docs: mermaid-init.js mermaid.min.js ## 📚 Generate the documentation
	mdbook build

docs-serve: mermaid-init.js mermaid.min.js ## 📚 Generate and serve the documentation
	mdbook serve --open

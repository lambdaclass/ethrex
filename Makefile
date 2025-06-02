.PHONY: build lint test clean run-image build-image clean-vectors \
		setup-hive test-pattern-default run-hive run-hive-debug clean-hive-logs \
		load-test-fibonacci load-test-io

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

build: ## 🔨 Build the client
	cargo build --workspace

lint: ## 🧹 Linter check
	cargo clippy --all-targets --all-features --workspace --exclude ethrex-replay --exclude ethrex-prover --exclude zkvm_interface -- -D warnings

CRATE ?= *
test: ## 🧪 Run each crate's tests
	cargo test -p '$(CRATE)' --workspace --exclude ethrex-levm --exclude ef_tests-blockchain --exclude ef_tests-state --exclude ethrex-l2 -- --skip test_contract_compilation
	$(MAKE) -C cmd/ef_tests/blockchain test

clean: clean-vectors ## 🧹 Remove build artifacts
	cargo clean
	rm -rf hive

STAMP_FILE := .docker_build_stamp
$(STAMP_FILE): $(shell find crates cmd -type f -name '*.rs') Cargo.toml Dockerfile
	docker build -t ethrex .
	touch $(STAMP_FILE)

build-image: $(STAMP_FILE) ## 🐳 Build the Docker image

run-image: build-image ## 🏃 Run the Docker image
	docker run --rm -p 127.0.0.1:8545:8545 ethrex --http.addr 0.0.0.0

dev: ## 🏃 Run the ethrex client in DEV_MODE with the InMemory Engine
	cargo run --bin ethrex --features dev -- \
			--network ./test_data/genesis-l1.json \
			--http.port 8545 \
			--http.addr 0.0.0.0 \
			--authrpc.port 8551 \
			--evm levm \
			--dev \
			--datadir memory

ETHEREUM_PACKAGE_REVISION := e73f52c34fd785700e9555aa41a78b0d5ca50173
# Shallow clones can't specify a single revision, but at least we avoid working
# the whole history by making it shallow since a given date (one day before our
# target revision).
ethereum-package:
	git clone --single-branch --branch ethrex-integration-pectra https://github.com/lambdaclass/ethereum-package

checkout-ethereum-package: ethereum-package ## 📦 Checkout specific Ethereum package revision
	cd ethereum-package && \
		git fetch && \
		git checkout $(ETHEREUM_PACKAGE_REVISION)

ENCLAVE ?= lambdanet

localnet: stop-localnet-silent build-image checkout-ethereum-package ## 🌐 Start local network
	kurtosis run --enclave $(ENCLAVE) ethereum-package --args-file test_data/network_params.yaml
	docker logs -f $$(docker ps -q --filter ancestor=ethrex)

localnet-assertoor-blob: stop-localnet-silent build-image checkout-ethereum-package ## 🌐 Start local network with assertoor test
	kurtosis run --enclave $(ENCLAVE) ethereum-package --args-file .github/config/assertoor/network_params_blob.yaml
	docker logs -f $$(docker ps -q --filter ancestor=ethrex)


localnet-assertoor-tx: stop-localnet-silent build-image checkout-ethereum-package ## 🌐 Start local network with assertoor test
	kurtosis run --enclave $(ENCLAVE) ethereum-package --args-file .github/config/assertoor/network_params_tx.yaml
	docker logs -f $$(docker ps -q --filter ancestor=ethrex)

stop-localnet: ## 🛑 Stop local network
	kurtosis enclave stop $(ENCLAVE)
	kurtosis enclave rm $(ENCLAVE) --force

stop-localnet-silent:
	@echo "Double checking local net is not already started..."
	@kurtosis enclave stop $(ENCLAVE) >/dev/null 2>&1 || true
	@kurtosis enclave rm $(ENCLAVE) --force >/dev/null 2>&1 || true

HIVE_BRANCH ?= master

hive:
	git clone --branch $(HIVE_BRANCH) https://github.com/lambdaclass/hive && \
	cd hive && \
	go build .; \

setup-hive: hive ## 🐝 Set up Hive testing framework
	cd hive && \
	git checkout $(HIVE_BRANCH) && \
	go build .;\

TEST_PATTERN ?= /
SIM_LOG_LEVEL ?= 4
SIM_PARALLELISM := 16

# Runs a hive testing suite and opens an web interface on http://127.0.0.1:8080
# The endpoints tested may be limited by supplying a test pattern in the form "/endpoint_1|enpoint_2|..|enpoint_n"
# For example, to run the rpc-compat suites for eth_chainId & eth_blockNumber you should run:
# `make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_blockNumber"`
run-hive: build-image setup-hive ## 🧪 Run Hive testing suite
	- cd hive && ./hive --client-file ../test_data/hive_clients.yml --client ethrex --sim $(SIMULATION) --sim.limit "$(TEST_PATTERN)" --sim.parallelism "$(SIM_PARALLELISM)"
	$(MAKE) view-hive

run-hive-all: build-image setup-hive ## 🧪 Run all Hive testing suites
	- cd hive && ./hive --client-file ../test_data/hive_clients.yml --client ethrex --sim ".*" --sim.parallelism "$(SIM_PARALLELISM)"
	$(MAKE) view-hive

run-hive-debug: build-image setup-hive ## 🐞 Run Hive testing suite in debug mode
	- cd hive && ./hive --sim $(SIMULATION) --client-file ../test_data/hive_clients.yml --client ethrex --sim.loglevel $(SIM_LOG_LEVEL) --sim.limit "$(TEST_PATTERN)" --sim.parallelism "$(SIM_PARALLELISM)" --docker.output
	$(MAKE) view-hive

run-hive-local: setup-hive
	- mkdir hive/clients/ethrex/ethrex && \ 
	cp -R cmd/ hive/clients/ethrex/ethrex/cmd/ && \
	cp -R crates/ hive/clients/ethrex/ethrex/crates/ && \
	cp -R tools/ hive/clients/ethrex/ethrex/tools/ && \
	cp -R test_data/ hive/clients/ethrex/ethrex/test_data/ && \
	cp Cargo.toml hive/clients/ethrex/ethrex/ && \
	cp Makefile hive/clients/ethrex/ethrex/ && \
	cd hive && ./hive --client-file ../test_data/hive_clients.yml --client ethrex --sim.loglevel 4 --sim.limit "/" --sim.parallelism "16" --docker.output --sim devp2p
	$(MAKE) view-hive

clean-hive-logs: ## 🧹 Clean Hive logs
	rm -rf ./hive/workspace/logs

view-hive: ## 🛠️ Builds hiveview with the logs from the hive execution
	cd hive && go build ./cmd/hiveview && ./hiveview --serve --logdir ./workspace/logs

install-cli: ## 🛠️ Installs the ethrex-l2 cli
	cargo install --path cmd/ethrex_l2/ --force --locked

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
	--features "dev" \
	--  \
	--evm $$LEVM \
	--network test_data/genesis-l2.json \
	--http.port 1729 \
	--dev \
	--datadir test_ethrex

load-test: ## 🚧 Runs a load-test. Run make start-node-with-flamegraph and in a new terminal make load-node
	cargo run --release --manifest-path ./cmd/load_test/Cargo.toml -- -k ./test_data/private_keys.txt -t eth-transfers

load-test-erc20:
	cargo run --release --manifest-path ./cmd/load_test/Cargo.toml -- -k ./test_data/private_keys.txt -t erc20

load-test-fibonacci:
	cargo run --release --manifest-path ./cmd/load_test/Cargo.toml -- -k ./test_data/private_keys.txt -t fibonacci

load-test-io:
	cargo run --release --manifest-path ./cmd/load_test/Cargo.toml -- -k ./test_data/private_keys.txt -t io-heavy

rm-test-db:  ## 🛑 Removes the DB used by the ethrex client used for testing
	sudo cargo run --release --bin ethrex -- removedb --force --datadir test_ethrex

test_data/ERC20/ERC20.bin: ## 🔨 Build the ERC20 contract for the load test
	solc ./test_data/ERC20.sol -o $@

sort-genesis-files:
	cd ./tools && cargo run

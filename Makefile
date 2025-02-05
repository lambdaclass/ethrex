.PHONY: build lint test clean run-image build-image download-test-vectors clean-vectors \
	setup-hive test-pattern-default run-hive run-hive-debug clean-hive-logs loc-detailed \
	loc-compare-detailed

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

build: ## 🔨 Build the client
	cargo build --workspace

lint: ## 🧹 Linter check
	cargo clippy --all-targets --all-features --workspace --exclude ethrex-prover -- -D warnings

SPECTEST_LINK := https://github.com/ethereum/execution-spec-tests/releases/download/pectra-devnet-6%40v1.0.0/fixtures_pectra-devnet-6.tar.gz
SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := cmd/ef_tests/ethrex/vectors

CRATE ?= *
test: $(SPECTEST_VECTORS_DIR) ## 🧪 Run each crate's tests
	cargo test -p '$(CRATE)' --workspace --exclude ethrex-prover --exclude ethrex-levm --exclude ef_tests-levm --exclude ethrex-l2 -- --skip test_contract_compilation

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

# This target is used to store the current version of EF tests downloaded.
# If the SPECTEST_LINK variable is modified, this target will be run and the file modified, which will trigger a new $(SPECTEST_ARTIFACT) build
# Thanks to:
# - https://www.technovelty.org/tips/the-stamp-idiom-with-make.html
# - https://stackoverflow.com/questions/27791578/makefile-how-to-detect-changes-within-the-makefile-itself
EF_TEST_STAMP_FILE := .ef_tests_stamp_file
$(EF_TEST_STAMP_FILE): Makefile
	echo $(SPECTEST_LINK) | cmp -s - $@ || echo $(SPECTEST_LINK) > $@

$(SPECTEST_ARTIFACT): $(EF_TEST_STAMP_FILE)
	rm -f tests.tar.gz # Delete older versions
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_LINK)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	mkdir -p $(SPECTEST_VECTORS_DIR) tmp
	tar -xzf $(SPECTEST_ARTIFACT) -C tmp
	rm -rf $(SPECTEST_VECTORS_DIR)/*
	mv tmp/fixtures/blockchain_tests/* $(SPECTEST_VECTORS_DIR)

download-test-vectors: $(SPECTEST_VECTORS_DIR) ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(SPECTEST_VECTORS_DIR)

ETHEREUM_PACKAGE_REVISION := 5b49d02ee556232a73ea1e28000ec5b3fca1073f
# Shallow clones can't specify a single revision, but at least we avoid working
# the whole history by making it shallow since a given date (one day before our
# target revision).
ethereum-package:
	git clone --single-branch --branch ethrex-integration https://github.com/lambdaclass/ethereum-package

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

HIVE_REVISION := b0b0f98bd24676239722e3aa7885e29ef856d804
# Shallow clones can't specify a single revision, but at least we avoid working
# the whole history by making it shallow since a given date (one day before our
# target revision).
HIVE_SHALLOW_SINCE := 2024-09-02
QUIET ?= false
# TODO change it back when we merge the PR.
# 		git clone --quiet --single-branch --branch master --shallow-since=$(HIVE_SHALLOW_SINCE) https://github.com/lambdaclass/hive ; \
# 		git clone --single-branch --branch master --shallow-since=$(HIVE_SHALLOW_SINCE) https://github.com/lambdaclass/hive ; \

hive:
	if [ "$(QUIET)" = "true" ]; then \
		git clone --quiet https://github.com/lambdaclass/hive; \
	else \
		git clone  https://github.com/lambdaclass/hive; \
	fi

setup-hive: hive ## 🐝 Set up Hive testing framework
	if [ "$$(cd hive && git rev-parse HEAD)" != "$(HIVE_REVISION)" ]; then \
		if [ "$(QUIET)" = "true" ]; then \
			cd hive && \
			git checkout --quiet master && \
			git fetch --quiet --shallow-since=$(HIVE_SHALLOW_SINCE) && \
			git checkout --quiet --detach $(HIVE_REVISION) && go build .;\
		else \
			cd hive && \
			git checkout master && \
			git fetch --shallow-since=$(HIVE_SHALLOW_SINCE) && \
			git checkout --detach $(HIVE_REVISION) && go build .;\
		fi \
	fi

TEST_PATTERN ?= /
SIM_LOG_LEVEL ?= 4

# Runs a hive testing suite
# The endpoints tested may be limited by supplying a test pattern in the form "/endpoint_1|enpoint_2|..|enpoint_n"
# For example, to run the rpc-compat suites for eth_chainId & eth_blockNumber you should run:
# `make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_blockNumber"`
run-hive: build-image setup-hive ## 🧪 Run Hive testing suite
	cd hive && ./hive --client ethrex --sim $(SIMULATION) --sim.limit "$(TEST_PATTERN)"

run-hive-levm: build-image setup-hive ## 🧪 Run Hive testing suite with LEVM
	cd hive && ./hive --client ethrex --ethrex.flags "--evm levm" --sim $(SIMULATION) --sim.limit "$(TEST_PATTERN)"

run-hive-all: build-image setup-hive ## 🧪 Run all Hive testing suites
	cd hive && ./hive --client ethrex --sim ".*" --sim.parallelism 4

run-hive-debug: build-image setup-hive ## 🐞 Run Hive testing suite in debug mode
	cd hive && ./hive --sim $(SIMULATION) --client ethrex --sim.loglevel $(SIM_LOG_LEVEL) --sim.limit "$(TEST_PATTERN)" --docker.output

clean-hive-logs: ## 🧹 Clean Hive logs
	rm -rf ./hive/workspace/logs

loc:
	cargo run -p loc

loc-stats:
	if [ "$(QUIET)" = "true" ]; then \
		cargo run --quiet -p loc -- --summary;\
	else \
		cargo run -p loc -- --summary;\
	fi

loc-detailed:
	cargo run --release --bin loc -- --detailed

loc-compare-detailed:
	cargo run --release --bin loc -- --compare-detailed

hive-stats:
	make hive QUIET=true
	make setup-hive QUIET=true
	rm -rf hive/workspace $(FILE_NAME)_logs
	make run-hive-all SIMULATION=ethereum/rpc-compat || exit 0
	make run-hive-all SIMULATION=devp2p || exit 0
	make run-hive-all SIMULATION=ethereum/engine || exit 0
	make run-hive-all SIMULATION=ethereum/sync || exit 0

stats:
	make loc-stats QUIET=true && echo
	cd crates/vm/levm && make download-evm-ef-tests
	cd crates/vm/levm && make run-evm-ef-tests QUIET=true && echo
	make hive-stats
	cargo run --quiet --release -p hive_report

install-cli: ## 🛠️ Installs the ethrex-l2 cli
	cargo install --path cmd/ethrex_l2/ --force

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
	--datadir test_ethrex

load-node: install-cli ## 🚧 Runs a load-test. Run make start-node-with-flamegraph and in a new terminal make load-node
	@if [ -z "$$C" ]; then \
		CONTRACT_INTERACTION=""; \
		echo "Running the load-test without contract interaction"; \
		echo "If you want to interact with contracts to load the evm, run the target with a C at the end: make <target> C=1"; \
	else \
		CONTRACT_INTERACTION="-c"; \
		echo "Running the load-test with contract interaction"; \
	fi; \
	ethrex_l2 test load --path test_data/private_keys.txt -i 1000 -v  --value 100000 $$CONTRACT_INTERACTION

rm-test-db:  ## 🛑 Removes the DB used by the ethrex client used for testing
	sudo cargo run --release --bin ethrex -- removedb --datadir test_ethrex

flamegraph: ## 🚧 Runs a load-test. Run make start-node-with-flamegraph and in a new terminal make flamegraph
	bash scripts/flamegraph.sh

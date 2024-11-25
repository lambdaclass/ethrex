.PHONY: build lint test clean run-image build-image download-test-vectors clean-vectors \
	setup-hive test-pattern-default run-hive run-hive-debug clean-hive-logs

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

build: ## 🔨 Build the client
	cargo build --workspace

lint: ## 🧹 Linter check
	cargo clippy --all-targets --all-features --workspace --exclude ethrex-prover -- -D warnings

SPECTEST_VERSION := v3.0.0
SPECTEST_ARTIFACT := tests_$(SPECTEST_VERSION).tar.gz
SPECTEST_VECTORS_DIR := cmd/ef_tests/ethrex/vectors

CRATE ?= *
test: $(SPECTEST_VECTORS_DIR) ## 🧪 Run each crate's tests
	cargo test -p '$(CRATE)' --workspace --exclude ethrex-prover --exclude ethrex-levm --exclude ef_tests-levm -- --skip test_contract_compilation --skip testito

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

$(SPECTEST_ARTIFACT):
	rm -f tests_*.tar.gz # Delete older versions
	curl -L -o $(SPECTEST_ARTIFACT) "https://github.com/ethereum/execution-spec-tests/releases/download/$(SPECTEST_VERSION)/fixtures_stable.tar.gz"

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	mkdir -p $(SPECTEST_VECTORS_DIR) tmp
	tar -xzf $(SPECTEST_ARTIFACT) -C tmp
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

localnet: stop-localnet-silent build-image checkout-ethereum-package ## 🌐 Start local network
	kurtosis run --enclave lambdanet ethereum-package --args-file test_data/network_params.yaml
	docker logs -f $$(docker ps -q --filter ancestor=ethrex)

stop-localnet: ## 🛑 Stop local network
	kurtosis enclave stop lambdanet
	kurtosis enclave rm lambdanet --force

stop-localnet-silent:
	@echo "Double checking local net is not already started..."
	@kurtosis enclave stop lambdanet >/dev/null 2>&1 || true
	@kurtosis enclave rm lambdanet --force >/dev/null 2>&1 || true

HIVE_REVISION := fc6ddec210095e2369019e7f4ab2f9f38e35a8e8
# Shallow clones can't specify a single revision, but at least we avoid working
# the whole history by making it shallow since a given date (one day before our
# target revision).
HIVE_SHALLOW_SINCE := 2024-09-02
hive:
	git clone --single-branch --branch master --shallow-since=$(HIVE_SHALLOW_SINCE) https://github.com/lambdaclass/hive
	cd hive && git checkout --detach $(HIVE_REVISION) && go build .

setup-hive: hive ## 🐝 Set up Hive testing framework
	if [ "$$(cd hive && git rev-parse HEAD)" != "$(HIVE_REVISION)" ]; then \
		cd hive && \
		git checkout master && \
		git fetch --shallow-since=$(HIVE_SHALLOW_SINCE) && \
		git checkout --detach $(HIVE_REVISION) && go build . ;\
	fi

TEST_PATTERN ?= /

# Runs a hive testing suite
# The endpoints tested may be limited by supplying a test pattern in the form "/endpoint_1|enpoint_2|..|enpoint_n"
# For example, to run the rpc-compat suites for eth_chainId & eth_blockNumber you should run:
# `make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_blockNumber"`
run-hive: build-image setup-hive ## 🧪 Run Hive testing suite
	cd hive && ./hive --sim $(SIMULATION) --client ethrex --sim.limit "$(TEST_PATTERN)"

run-hive-on-latest: setup-hive ## 🧪 Run Hive testing suite with the latest docker image
	cd hive && ./hive --sim $(SIMULATION) --client ethrex --sim.limit "$(TEST_PATTERN)"

run-hive-debug: build-image setup-hive ## 🐞 Run Hive testing suite in debug mode
	cd hive && ./hive --sim $(SIMULATION) --client ethrex --sim.limit "$(TEST_PATTERN)" --docker.output

clean-hive-logs: ## 🧹 Clean Hive logs
	rm -rf ./hive/workspace/logs

loc:
	cargo run -p loc

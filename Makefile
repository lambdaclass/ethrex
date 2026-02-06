.PHONY: build lint test clean run-image build-image clean-vectors \
		setup-hive test-pattern-default run-hive run-hive-debug clean-hive-logs \
		load-test-fibonacci load-test-io run-hive-eels-blobs \
		build-bolt bolt-instrument bolt-optimize bolt-clean \
		bolt-perf2bolt bolt-profile bolt-verify bolt-full \
		pgo-bolt-build pgo-bolt-optimize pgo-full-build pgo-full-optimize

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

# Frame pointers for profiling (default off, set FRAME_POINTERS=1 to enable)
FRAME_POINTERS ?= 0

ifeq ($(FRAME_POINTERS),1)
PROFILING_CFG := --config .cargo/profiling.toml
endif

build: ## 🔨 Build the client
	cargo build $(PROFILING_CFG) --workspace

# BOLT optimization targets (Linux x86-64 only)
# Prerequisites: llvm-bolt from LLVM 19+
# See docs/developers/bolt-optimization.md for full setup instructions.
#
# BOLT flags (--emit-relocs, frame pointers) are isolated in .cargo/bolt.toml
# and only loaded by the build-bolt target, so normal builds are unaffected.
#
BOLT_PROFILE_DIR ?= /tmp/bolt-profiles
BOLT_BINARY := target/release-bolt/ethrex
BOLT_GENESIS ?= fixtures/genesis/perf-ci.json
BOLT_BLOCKS ?= fixtures/blockchain/l2-1k-erc20.rlp
PERF_DATA ?= perf.data

# Verify BOLT prerequisites before doing anything
bolt-check:
	@uname -m | grep -q x86_64 || { echo "ERROR: BOLT requires x86_64 (current: $$(uname -m))"; exit 1; }
	@uname -s | grep -q Linux || { echo "ERROR: BOLT requires Linux (current: $$(uname -s))"; exit 1; }
	@command -v llvm-bolt >/dev/null 2>&1 || { echo "ERROR: llvm-bolt not found. See docs/developers/bolt-optimization.md for install instructions."; exit 1; }
	@if [ ! -s $(BOLT_BLOCKS) ]; then \
		echo "ERROR: $(BOLT_BLOCKS) missing or empty. Run 'git lfs pull' to fetch fixture files."; \
		exit 1; \
	fi

build-bolt: bolt-check ## 🔨 Build release binary for BOLT optimization (with relocations)
	CXXFLAGS='-fno-reorder-blocks-and-partition' cargo build --profile release-bolt --config .cargo/bolt.toml

bolt-perf2bolt: ## 📊 Convert perf.data to BOLT profile format
	@mkdir -p $(BOLT_PROFILE_DIR)
	perf2bolt -p $(PERF_DATA) -o $(BOLT_PROFILE_DIR)/perf.fdata $(BOLT_BINARY)

bolt-optimize: ## ⚡ Apply BOLT optimization using collected profiles
	@if [ -f $(BOLT_PROFILE_DIR)/perf.fdata ]; then \
		llvm-bolt $(BOLT_BINARY) -o ethrex-bolt-optimized \
			-data=$(BOLT_PROFILE_DIR)/perf.fdata \
			-reorder-blocks=ext-tsp \
			-reorder-functions=cdsort \
			-split-functions \
			-split-all-cold \
			-split-eh \
			-icf=1 \
			-use-gnu-stack \
			-dyno-stats; \
	elif ls $(BOLT_PROFILE_DIR)/prof.* 1>/dev/null 2>&1; then \
		merge-fdata $(BOLT_PROFILE_DIR)/prof.* > $(BOLT_PROFILE_DIR)/merged.fdata && \
		llvm-bolt $(BOLT_BINARY) -o ethrex-bolt-optimized \
			-data=$(BOLT_PROFILE_DIR)/merged.fdata \
			-reorder-blocks=ext-tsp \
			-reorder-functions=cdsort \
			-split-functions \
			-split-all-cold \
			-split-eh \
			-icf=1 \
			-use-gnu-stack \
			-dyno-stats; \
	else \
		echo "No profile data found. Run perf profiling first:"; \
		echo "  perf record -e cycles:u -j any,u -o perf.data -- $(BOLT_BINARY) ..."; \
		echo "  make bolt-perf2bolt"; \
		exit 1; \
	fi

bolt-verify: ## 🔍 Verify binary was processed by BOLT
	@if readelf -S ethrex-bolt-optimized 2>/dev/null | grep -q '\.note\.bolt_info'; then \
		echo "✓ Binary contains BOLT markers"; \
		readelf -p .note.bolt_info ethrex-bolt-optimized 2>/dev/null || true; \
	else \
		echo "✗ Binary does not contain BOLT markers"; \
		exit 1; \
	fi

bolt-clean: ## 🧹 Clean BOLT profiles and artifacts
	rm -rf $(BOLT_PROFILE_DIR)
	rm -f ethrex-instrumented ethrex-bolt-optimized $(PERF_DATA)

# BOLT instrumentation (creates instrumented binary for profiling)
bolt-instrument: build-bolt ## 🔧 Create BOLT-instrumented binary for profiling
	@mkdir -p $(BOLT_PROFILE_DIR)
	llvm-bolt \
		$(BOLT_BINARY) \
		-o ethrex-instrumented \
		-instrument \
		--instrumentation-file-append-pid \
		--instrumentation-file=$(BOLT_PROFILE_DIR)/prof
	@echo "Instrumented binary created: ethrex-instrumented"
	@echo "Run 'make bolt-profile' to collect profile data, or run the binary manually."

bolt-profile: ## 📊 Run instrumented binary with benchmark blocks to collect profile data
	@test -f ethrex-instrumented || { echo "ERROR: Run 'make bolt-instrument' first."; exit 1; }
	@rm -rf /tmp/bolt-data $(BOLT_PROFILE_DIR)/prof.*
	@echo "Profiling with $(BOLT_BLOCKS) (this may take a few minutes)..."
	./ethrex-instrumented \
		--network $(BOLT_GENESIS) \
		--datadir /tmp/bolt-data \
		import $(BOLT_BLOCKS)
	@rm -rf /tmp/bolt-data
	@echo "Profile data collected:"
	@ls -lh $(BOLT_PROFILE_DIR)/prof.*

bolt-full: bolt-instrument bolt-profile bolt-optimize bolt-verify ## 🚀 Full BOLT workflow: build → instrument → profile → optimize → verify
	@echo ""
	@echo "BOLT optimization complete. Optimized binary: ethrex-bolt-optimized"
	@echo "Benchmark with: make bolt-bench"

bolt-bench: ## 📈 Benchmark baseline vs BOLT-optimized binary
	@test -f ethrex-bolt-optimized || { echo "ERROR: Run 'make bolt-full' or 'make bolt-optimize' first."; exit 1; }
	@echo "=== Baseline (3 runs) ==="
	@for i in 1 2 3; do \
		rm -rf /tmp/bolt-bench-db; \
		$(BOLT_BINARY) \
			--network $(BOLT_GENESIS) \
			--datadir /tmp/bolt-bench-db \
			import $(BOLT_BLOCKS) 2>&1 | grep "Import completed"; \
	done
	@echo ""
	@echo "=== BOLT-optimized (3 runs) ==="
	@for i in 1 2 3; do \
		rm -rf /tmp/bolt-bench-db; \
		./ethrex-bolt-optimized \
			--network $(BOLT_GENESIS) \
			--datadir /tmp/bolt-bench-db \
			import $(BOLT_BLOCKS) 2>&1 | grep "Import completed"; \
	done
	@rm -rf /tmp/bolt-bench-db

# cargo-pgo workflow (requires: cargo install cargo-pgo)
# NOTE: cargo-pgo doesn't pass CXXFLAGS, so use the manual Makefile targets instead
pgo-bolt-build: ## 🔨 Build with cargo-pgo for BOLT instrumentation (use build-bolt instead)
	@echo "NOTE: cargo-pgo doesn't pass CXXFLAGS. Use 'make build-bolt' then 'make bolt-instrument' instead."
	CXXFLAGS='-fno-reorder-blocks-and-partition' cargo pgo bolt build --release

pgo-bolt-optimize: ## ⚡ Build BOLT-optimized binary with cargo-pgo
	cargo pgo bolt optimize --release

pgo-full-build: ## 🔨 Build with PGO instrumentation
	cargo pgo build

pgo-full-optimize: ## ⚡ Build PGO+BOLT optimized binary
	CXXFLAGS='-fno-reorder-blocks-and-partition' cargo pgo bolt build --with-pgo
	@echo "Run profiling workload, then: cargo pgo bolt optimize --with-pgo"

lint-l1:
	cargo clippy --lib --bins -F debug,sync-test \
		--release -- -D warnings

lint-l2:
	cargo clippy --all-targets -F debug,sync-test,l2,l2-sql \
		--workspace --exclude ethrex-prover --exclude ethrex-guest-program \
		--release -- -D warnings

lint-gpu:
	cargo clippy --all-targets -F debug,sync-test,l2,l2-sql,,sp1,risc0,gpu \
		--workspace --exclude ethrex-prover --exclude ethrex-guest-program \
		--release -- -D warnings

lint: lint-l1 lint-l2 ## 🧹 Linter check

CRATE ?= *
# CAUTION: It is important that the ethrex-l2 crate remains excluded here,
# as its tests depend on external setup that is not handled by this Makefile.
test: ## 🧪 Run each crate's tests
	cargo test $(PROFILING_CFG) -p '$(CRATE)' --workspace --exclude ethrex-l2

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
	cargo run $(PROFILING_CFG) --release -- \
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
	cargo run $(PROFILING_CFG) --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t eth-transfers

load-test-erc20:
	cargo run $(PROFILING_CFG) --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t erc20

load-test-fibonacci:
	cargo run $(PROFILING_CFG) --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t fibonacci

load-test-io:
	cargo run $(PROFILING_CFG) --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t io-heavy

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
	cargo tree --manifest-path crates/guest-program/bin/sp1/Cargo.toml
	cargo tree --manifest-path crates/guest-program/bin/risc0/Cargo.toml
	cargo tree --manifest-path crates/guest-program/bin/zisk/Cargo.toml
	cargo tree --manifest-path crates/guest-program/bin/openvm/Cargo.toml
	cargo tree --manifest-path crates/l2/tee/quote-gen/Cargo.toml
	cargo tree --manifest-path crates/vm/levm/bench/revm_comparison/Cargo.toml
	cargo tree --manifest-path tooling/Cargo.toml
	cargo tree --manifest-path tooling/ef_tests/state/Cargo.toml

check-cargo-lock: ## 🔍 Check Cargo.lock files are up to date
	cargo metadata --locked > /dev/null
	cargo metadata --locked --manifest-path crates/guest-program/bin/sp1/Cargo.toml > /dev/null
	cargo metadata --locked --manifest-path crates/guest-program/bin/risc0/Cargo.toml > /dev/null
	# We use metadata so we don't need to have the ZisK toolchain installed and verify compilation
	# if changes made to the source code CI will run with the toolchain
	cargo metadata --locked --manifest-path crates/guest-program/bin/zisk/Cargo.toml > /dev/null
	cargo metadata --locked --manifest-path crates/guest-program/bin/openvm/Cargo.toml > /dev/null
	cargo metadata --locked --manifest-path crates/l2/tee/quote-gen/Cargo.toml > /dev/null
	cargo metadata --locked --manifest-path crates/vm/levm/bench/revm_comparison/Cargo.toml > /dev/null
	cargo metadata --locked --manifest-path tooling/Cargo.toml > /dev/null
	cargo metadata --locked --manifest-path tooling/ef_tests/state/Cargo.toml > /dev/null

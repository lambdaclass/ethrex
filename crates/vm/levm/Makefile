.PHONY: all test clippy fmt usage lint eth-tests run-evm-ef-tests flamegraph-run-ef-tests samply-run-ef-tests

all: test clippy fmt ## 🚀 Runs all tests, linter and formatter

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

test: ## 🧪 Runs all tests except Ethereum tests
	cargo test -p ethrex-levm

lint: ## 🧹 Linter check
	cargo clippy --all-targets --all-features -- -D warnings

fmt: ## 📄 Runs rustfmt
	cargo fmt --all

###### EF Tests ######
EFTEST_DIR := ../../../cmd/ef_tests/levm
VECTORS_DIR := $(EFTEST_DIR)/vectors
TMP_DIR := tmp
TESTS_REPO := $(TMP_DIR)/ethereum-tests

ETH_TEST_URL := https://github.com/ethereum/tests.git
ETH_TEST_TAG := v14.1
COMMIT_LEGACY_TESTS_FOR_TAG := b2e6c9e

STATETEST_VERSION := pectra-devnet-6%40v1.0.0
STATETEST_NET := pectra-devnet-6
STATETEST_ARTIFACT := fixtures_$(STATETEST_NET).tar.gz
STATETEST_URL := https://github.com/ethereum/execution-spec-tests/releases/download/$(STATETEST_VERSION)/fixtures_$(STATETEST_NET).tar.gz

setup-test-dirs:
	mkdir -p $(VECTORS_DIR)
	mkdir -p $(VECTORS_DIR)/LegacyTests/Constantinople/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/state_tests

clone-ef-tests: ## 📥 Download Ethereum Tests repository with submodules
	mkdir -p $(TMP_DIR)
	git clone --recurse-submodules --depth 1 --branch $(ETH_TEST_TAG) $(ETH_TEST_URL) $(TESTS_REPO)
	cd $(TESTS_REPO)/LegacyTests && git checkout $(COMMIT_LEGACY_TESTS_FOR_TAG)
	cp -r $(TESTS_REPO)/GeneralStateTests/* $(VECTORS_DIR)/GeneralStateTests/
	cp -r $(TESTS_REPO)/LegacyTests/Constantinople/GeneralStateTests/* $(VECTORS_DIR)/LegacyTests/Constantinople/GeneralStateTests/;

download-state-tests: ## 📥 Download and setup state tests fixtures
	curl -L -o $(STATETEST_ARTIFACT) $(STATETEST_URL)
	tar -xzf $(STATETEST_ARTIFACT) -C $(TMP_DIR)
	mv $(TMP_DIR)/fixtures/state_tests/* $(VECTORS_DIR)/state_tests/
	rm -f $(STATETEST_ARTIFACT)

download-evm-ef-tests: setup-test-dirs clone-ef-tests download-state-tests ## 📥 Download and setup all EF Tests
	rm -rf $(TMP_DIR)

clean-evm-ef-tests: ## 🗑️ Clean test vectors and temporary files
	rm -rf $(VECTORS_DIR)
	rm -rf $(TMP_DIR)
	rm -f $(STATETEST_ARTIFACT)

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	if [ "$(QUIET)" = "true" ]; then \
		cd ../../../ && \
		time cargo test --quiet -p ef_tests-levm --test ef_tests_levm --release -- $(flags) --summary;\
	elif [ "$(DEBUG)" = "true" ]; then \
		cd ../../../ && \
		time cargo test -p ef_tests-levm --test ef_tests_levm -- $(flags);\
	else \
		cd ../../../ && \
		time cargo test -p ef_tests-levm --test ef_tests_levm --release -- $(flags);\
	fi

run-evm-ef-tests-ci: ## 🏃‍♂️ Run EF Tests only with LEVM and without spinner, for CI.
	cd ../../../ && \
	time cargo test -p ef_tests-levm --test ef_tests_levm --release -- --summary

generate-evm-ef-tests-report: ## 📊 Generate EF Tests Report
	cd ../../../ && \
	cargo test -p ef_tests-levm --test ef_tests_levm --release -- --summary

###### Benchmarks ######
REPETITIONS := 10000
REPETITIONS_SLOW := 200

BENCH_FACT_ITERATIONS := 57
BENCH_FACT_REC_ITERATIONS := 57
BENCH_FIB_ITERATIONS := 57
BENCH_HASHES_ITERATIONS := 57
BENCH_BUBBLESORT_ITERATIONS := 100 # Size of the array to sort
BENCH_MINT_ITERATIONS := 500
BENCH_TRANSFER_ITERATIONS := 500
BENCH_APPROVAL_ITERATIONS := 500

define run_benchmark
	@printf "%s\n" "revm_$(1)"
	@target/release/benchmark revm $(1) 1 $($(3))
	@printf "%s\n" "levm_$(1)"
	@target/release/benchmark levm $(1) 1 $($(3))
	hyperfine -w 5 -r 10 -N \
		-n "revm_$(1)" "target/release/benchmark revm $(1) $($(2)) $($(3))" \
		-n "levm_$(1)" "target/release/benchmark levm $(1) $($(2)) $($(3))"
	@echo
endef

define run_benchmark_ci
	@printf "%s\n" "revm_$(1)"
	@target/release/benchmark revm $(1) 1 $($(3))
	@printf "%s\n" "levm_$(1)"
	@target/release/benchmark levm $(1) 1 $($(3))
	hyperfine -w 5 -r 10 -N --export-markdown $(1).md \
		-n "revm_$(1)" "target/release/benchmark revm $(1) $($(2)) $($(3))" \
		-n "levm_$(1)" "target/release/benchmark levm $(1) $($(2)) $($(3))"
	@echo
endef

compile-contracts:
	cd ../../../ && \
	cargo run --package revm_comparison \
	--bin compile \
	--manifest-path crates/vm/levm/bench/revm_comparison/Cargo.toml

revm-comparison: compile-contracts ## 📊 Run benchmarks of fibonacci and factorial for both REVM and LEVM
	$(MAKE) build-revm-comparison
	$(call run_benchmark,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_benchmark,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_benchmark,FactorialRecursive,REPETITIONS,BENCH_FACT_REC_ITERATIONS)
	$(call run_benchmark,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_benchmark,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_benchmark,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_benchmark,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_benchmark,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

revm-comparison-ci: compile-contracts
	$(MAKE) build-revm-comparison
	$(call run_benchmark_ci,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_benchmark_ci,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_benchmark_ci,FactorialRecursive,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_benchmark_ci,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_benchmark_ci,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_benchmark_ci,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_benchmark_ci,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_benchmark_ci,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

build-revm-comparison:
	cd bench/revm_comparison && \
		CARGO_TARGET_DIR=../../target \
		cargo build --release --bin benchmark

###### Build Client with LEVM ######

FLAGS := "--features ethrex-blockchain/levm,ethrex-vm/levm,ethrex/levm"

build-image-levm: ## 🐳 Build the Docker image with LEVM features
	cd ../../../ && \
	docker build -t ethrex --build-arg BUILD_FLAGS=$(FLAGS) .

run-hive-debug-levm: build-image-levm ## 🐝 Run Hive with LEVM in debug mode
	$(MAKE) -C ../../../ setup-hive
	cd ../../../hive && ./hive --sim ethereum/rpc-compat --client ethrex --sim.limit "$(TEST_PATTERN)" --docker.output || exit 0
	cd ../../../hive && ./hive --sim devp2p --client ethrex --sim.limit "$(TEST_PATTERN)" --docker.output || exit 0
	cd ../../../hive && ./hive --sim ethereum/engine --client ethrex --sim.limit "$(TEST_PATTERN)" --docker.output || exit 0
	cd ../../../hive && ./hive --sim ethereum/sync --client ethrex --sim.limit "$(TEST_PATTERN)" --docker.output || exit 0
	cargo run --release -p hive_report

SUBDIRS := $(shell find $(VECTORS_DIR)/GeneralStateTests -maxdepth 1 -type d ! -path "$(VECTORS_DIR)/GeneralStateTests" -exec basename {} \;)

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	cd $(EFTEST_DIR) &&\
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/levm ||: && \
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/revm ||:
	$(MAKE) flamegraph-run-ef-tests-revm
	$(MAKE) flamegraph-run-ef-tests-levm

flamegraph-run-ef-tests-revm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output $(EFTEST_DIR)/levm_perfgraphs/flamegraph/ef_tests/revm/$$dir.svg\
		-p ef_tests-levm --test ef_tests_levm -- --summary --revm --tests $$dir;\
	done
flamegraph-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output $(EFTEST_DIR)/levm_perfgraphs/flamegraph/ef_tests/levm/$$dir.svg\
		-p ef_tests-levm --test ef_tests_levm -- --summary --tests $$dir;\
	done

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	cd $(EFTEST_DIR) && \
	mkdir -p levm_perfgraphs/samply/ef_tests/levm ||: && \
	mkdir -p levm_perfgraphs/samply/ef_tests/revm ||:
	$(MAKE) samply-run-ef-tests-revm
	$(MAKE) samply-run-ef-tests-levm

samply-run-ef-tests-revm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o $(EFTEST_DIR)/levm_perfgraphs/samply/ef_tests/revm/prof_$$dir.json \
		cargo test --release -p ef_tests-levm --test ef_tests_levm -- --summary --revm --tests $$dir;\
	done

samply-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o $(EFTEST_DIR)/levm_perfgraphs/samply/ef_tests/levm/prof_$$dir.json \
		cargo test --release -p ef_tests-levm --test ef_tests_levm -- --summary --tests $$dir;\
	done

################
# FLAMEGRAPHS
################
define run_flamegraph
	# revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_$(1).svg \
	-p revm_comparison --bin benchmark -- revm $(1) $($(2)) $($(3))
	# levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_$(1).svg \
	-p revm_comparison --bin benchmark -- levm $(1) $($(2)) $($(3))
endef

define run_samply
	# revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_revm_$(1).json \
	cargo run --release -p revm_comparison --bin benchmark -- revm $(1) $($(2)) $($(3))
	# levm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_$(1).json \
	cargo run --release -p revm_comparison --bin benchmark -- levm $(1) $($(2)) $($(3))
endef

FLAMEGRAPH_DIR := $(EFTEST_DIR)/levm_perfgraphs/flamegraph/bench
flamegraph-benchmarks: ## 🔥 Run benchmarks and create flamegraph
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/flamegraph/bench ||:
	$(call run_flamegraph,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_flamegraph,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_flamegraph,FactorialRecursive,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_flamegraph,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_flamegraph,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_flamegraph,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_flamegraph,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_flamegraph,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

SAMPLY_DIR := ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/bench
samply-benchmarks: ## ⚡️ Run benchmarks and create samply profiling file
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/samply/bench ||:
	$(call run_samply,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_samply,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_samply,FactorialRecursive,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_samply,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_samply,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_samply,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_samply,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_samply,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

.PHONY: download-evm-ef-tests clean-evm-ef-tests run-evm-ef-tests test-levm test-revm run-evm-ef-tests flamegraph-run-ef-tests samply-run-ef-tests amsterdam-vectors

FIXTURES_FILE := .fixtures_url
STATETEST_ARTIFACT := test.tar.gz
VECTORS_DIR := vectors

TMP_DIR := tmp
TESTS_REPO := $(TMP_DIR)/ethereum-tests

ETH_TEST_URL := https://github.com/ethereum/tests.git
ETH_TEST_TAG := v17.0
COMMIT_LEGACY_TESTS_FOR_TAG := b3f67fe

STATETEST_URL := $(shell cat $(FIXTURES_FILE))

AMSTERDAM_FIXTURES_FILE := .fixtures_url_amsterdam
AMSTERDAM_ARTIFACT := amsterdam-tests.tar.gz
AMSTERDAM_URL := $(shell cat $(AMSTERDAM_FIXTURES_FILE))

$(STATETEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-evm-ef-tests
	curl -L -o $(STATETEST_ARTIFACT) $(STATETEST_URL)

$(VECTORS_DIR): $(STATETEST_ARTIFACT)
	$(MAKE) setup-test-dirs
	$(MAKE) clone-ef-tests
	tar -xzf $(STATETEST_ARTIFACT) --strip-components=2 -C $(VECTORS_DIR)/state_tests fixtures/state_tests
	rm -f $(STATETEST_ARTIFACT)
	rm -rf $(TMP_DIR)

$(AMSTERDAM_ARTIFACT): $(AMSTERDAM_FIXTURES_FILE)
	curl -L -o $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_URL)

amsterdam-vectors: $(AMSTERDAM_ARTIFACT) $(VECTORS_DIR)
	tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(VECTORS_DIR)/state_tests fixtures/state_tests/amsterdam

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

setup-test-dirs:
	mkdir -p $(VECTORS_DIR)
	mkdir -p $(VECTORS_DIR)/LegacyTests/Cancun/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/state_tests

clone-ef-tests: ## 📥 Download Ethereum Tests repository with submodules
	mkdir -p $(TMP_DIR)
	git clone --recurse-submodules --depth 1 --branch $(ETH_TEST_TAG) $(ETH_TEST_URL) $(TESTS_REPO)
	cd $(TESTS_REPO)/LegacyTests && git checkout $(COMMIT_LEGACY_TESTS_FOR_TAG)
	cp -r $(TESTS_REPO)/GeneralStateTests/* $(VECTORS_DIR)/GeneralStateTests/
	cp -r $(TESTS_REPO)/LegacyTests/Cancun/GeneralStateTests/* $(VECTORS_DIR)/LegacyTests/Cancun/GeneralStateTests/;

download-evm-ef-tests: $(VECTORS_DIR) amsterdam-vectors ## 📥 Download and setup state tests fixtures

clean-evm-ef-tests: ## 🗑️ Clean test vectors and temporary files
	rm -rf $(VECTORS_DIR)
	rm -rf $(TMP_DIR)
	rm -f $(STATETEST_ARTIFACT) $(AMSTERDAM_ARTIFACT)

refresh-evm-ef-tests: clean-evm-ef-tests download-evm-ef-tests ## Cleans and re-downloads tests, useful when they are outdated!

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	if [ "$(QUIET)" = "true" ]; then \
		time cargo test --quiet --test all --profile release-with-debug -- $(flags) --summary;\
	elif [ "$(DEBUG)" = "true" ]; then \
		time cargo test --test all -- $(flags);\
	else \
		time cargo test --test all --profile release-with-debug -- $(flags);\
	fi

run-evm-ef-tests-ci: $(VECTORS_DIR) ## 🏃‍♂️ Run EF Tests only with LEVM and without spinner, for CI.
	time cargo test -p ef_tests-state --test all --profile release-with-debug -- --summary

test-levm: $(VECTORS_DIR)
	$(MAKE) run-evm-ef-tests flags="--summary"

test-revm: $(VECTORS_DIR)
	$(MAKE) run-evm-ef-tests flags="--revm"

###### Running Flamegraphs ######

SUBDIRS := $(shell find $(VECTORS_DIR)/GeneralStateTests -maxdepth 1 -type d ! -path "$(VECTORS_DIR)/GeneralStateTests" -exec basename {} \;)

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/state ||: && \
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/revm ||:
	$(MAKE) flamegraph-run-ef-tests-revm
	$(MAKE) flamegraph-run-ef-tests-levm

flamegraph-run-ef-tests-revm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output levm_perfgraphs/flamegraph/ef_tests/revm/$$dir.svg\
		-p ef_tests-state --test all -- --summary --revm --tests $$dir;\
	done

flamegraph-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output levm_perfgraphs/flamegraph/ef_tests/state/$$dir.svg\
		-p ef_tests-state --test all -- --summary --tests $$dir;\
	done

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	mkdir -p levm_perfgraphs/samply/ef_tests/state ||: && \
	mkdir -p levm_perfgraphs/samply/ef_tests/revm ||:
	$(MAKE) samply-run-ef-tests-revm
	$(MAKE) samply-run-ef-tests-levm

samply-run-ef-tests-revm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o levm_perfgraphs/samply/ef_tests/revm/prof_$$dir.json \
		cargo test --profile release-with-debug -p ef_tests-state --test all -- --summary --revm --tests $$dir;\
	done

samply-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o levm_perfgraphs/samply/ef_tests/state/prof_$$dir.json \
		cargo test --profile release-with-debug -p ef_tests-state --test all -- --summary --tests $$dir;\
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
	cargo run --profile release-with-debug -p revm_comparison --bin benchmark -- revm $(1) $($(2)) $($(3))
	# levm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_$(1).json \
	cargo run --profile release-with-debug -p revm_comparison --bin benchmark -- levm $(1) $($(2)) $($(3))
endef

FLAMEGRAPH_DIR := levm_perfgraphs/flamegraph/bench
flamegraph-benchmarks: ## 🔥 Run benchmarks and create flamegraph
	mkdir -p levm_perfgraphs/flamegraph/bench ||:
	$(call run_flamegraph,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_flamegraph,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_flamegraph,FactorialRecursive,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_flamegraph,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_flamegraph,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_flamegraph,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_flamegraph,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_flamegraph,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

SAMPLY_DIR := levm_perfgraphs/samply/bench
samply-benchmarks: ## ⚡️ Run benchmarks and create samply profiling file
	mkdir -p levm_perfgraphs/samply/bench ||:
	$(call run_samply,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_samply,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_samply,FactorialRecursive,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_samply,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_samply,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_samply,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_samply,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_samply,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

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

SPECTEST_VERSION := v14.1
SPECTEST_ARTIFACT := tests_$(SPECTEST_VERSION).tar.gz
SPECTEST_URL := https://github.com/ethereum/tests/archive/refs/tags/$(SPECTEST_VERSION).tar.gz

STATETEST_VERSION := pectra-devnet-5%40v1.1.0
STATETEST_NET := pectra-devnet-5
STATETEST_ARTIFACT := fixtures_$(STATETEST_NET).tar.gz
STATETEST_URL := https://github.com/ethereum/execution-spec-tests/releases/download/$(STATETEST_VERSION)/fixtures_$(STATETEST_NET).tar.gz

BENCH_FACT_REPETITIONS := 100000
BENCH_FACT_ITERATIONS := 57

BENCH_FIB_REPETITIONS := 100000
BENCH_FIB_ITERATIONS := 150

BENCH_HASHES_REPETITIONS := 10000
BENCH_HASHES_ITERATIONS := 100

BENCH_MINT_REPETITIONS := 10000
BENCH_MINT_ITERATIONS := 1000

BENCH_TRANSFER_REPETITIONS := 100
BENCH_TRANSFER_ITERATIONS := 500

BENCH_APPROVAL_REPETITIONS := 10000
BENCH_APPROVAL_ITERATIONS := 2000

download-evm-ef-tests: ## 📥 Download EF Tests
	mkdir -p $(TMP_DIR)
	mkdir -p $(VECTORS_DIR)/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/state_tests
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_URL)
	tar -xzf $(SPECTEST_ARTIFACT) -C $(TMP_DIR)
	mv $(TMP_DIR)/tests-14.1/GeneralStateTests/* $(VECTORS_DIR)/GeneralStateTests/
	curl -L -o $(STATETEST_ARTIFACT) $(STATETEST_URL)
	tar -xzf $(STATETEST_ARTIFACT) -C $(TMP_DIR)
	mv $(TMP_DIR)/fixtures/state_tests/* $(VECTORS_DIR)/state_tests/
	rm -rf $(TMP_DIR) tests_*.tar.gz
	rm -rf $(TMP_DIR) fixtures_*.tar.gz

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

clean-evm-ef-tests: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_DIR)

###### Benchmarks ######

compile-contracts:
	cd ../../../ && \
	cargo run --package revm_comparison \
	--bin compile \
	--manifest-path crates/vm/levm/bench/revm_comparison/Cargo.toml

revm-comparison: compile-contracts ## 📊 Run benchmarks of fibonacci and factorial for both REVM and LEVM
	$(MAKE) build-revm-comparison
	@echo
	# factorial
	@printf "%s\n" "revm_factorial"
	@target/release/benchmark revm Factorial 1 $(BENCH_FACT_ITERATIONS)
	@printf "%s\n" "levm_factorial"
	@target/release/benchmark levm Factorial 1 $(BENCH_FACT_ITERATIONS)
	hyperfine -w 5 -r 10 -N \
		-n "revm_factorial" "target/release/benchmark revm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)" \
		-n "levm_factorial" "target/release/benchmark levm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)"
	@echo
	# fibonacci
	@printf "%s\n" "revm_fibonacci"
	@target/release/benchmark revm Fibonacci 1 $(BENCH_FACT_ITERATIONS)
	@printf "%s\n" "levm_fibonacci"
	@target/release/benchmark levm Fibonacci 1 $(BENCH_FACT_ITERATIONS)
	hyperfine -w 5 -r 10 -N \
		-n "revm_fibonacci" "target/release/benchmark revm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)" \
		-n "levm_fibonacci" "target/release/benchmark levm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)"
	@echo
	# many_hashes
	@printf "%s\n" "revm_many_hashes"
	target/release/benchmark revm ManyHashes 1 $(BENCH_HASHES_ITERATIONS)
	@printf "%s\n" "levm_many_hashes"
	target/release/benchmark levm ManyHashes 1 $(BENCH_HASHES_ITERATIONS)
	hyperfine -w 5 -r 10 -N \
		-n "revm_many_hashes" "target/release/benchmark revm ManyHashes $(BENCH_HASHES_REPETITIONS) $(BENCH_HASHES_ITERATIONS)" \
		-n "levm_many_hashes" "target/release/benchmark levm ManyHashes $(BENCH_HASHES_REPETITIONS) $(BENCH_HASHES_ITERATIONS)"
	@echo
	# erc20_approval
	@printf "%s\n" "revm_erc20_approval"
	@target/release/benchmark revm ERC20ApprovalTransfer 1 $(BENCH_APPROVAL_ITERATIONS)
	@printf "%s\n" "levm_erc20_approval"
	@target/release/benchmark levm ERC20ApprovalTransfer 1 $(BENCH_APPROVAL_ITERATIONS)
	hyperfine -w 5 -r 10 -N \
		-n "revm_erc20_approval" "target/release/benchmark revm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)" \
		-n "levm_erc20_approval" "target/release/benchmark levm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)"
	@echo
	# erc20_transfer
	@printf "%s\n" "revm_erc20_transfer"
	@target/release/benchmark revm ERC20Transfer 1 $(BENCH_TRANSFER_ITERATIONS)
	@printf "%s\n" "levm_erc20_transfer"
	@target/release/benchmark levm ERC20Transfer 1 $(BENCH_TRANSFER_ITERATIONS)
	hyperfine -w 5 -r 10 -N \
		-n "revm_erc20_transfer" "target/release/benchmark revm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)" \
		-n "levm_erc20_transfer" "target/release/benchmark levm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)"
	@echo
	# erc20_mint
	@printf "%s\n" "revm_erc20_mint"
	@target/release/benchmark revm ERC20Mint 1 $(BENCH_MINT_ITERATIONS)
	@printf "%s\n" "levm_erc20_mint"
	@target/release/benchmark levm ERC20Mint 1 $(BENCH_MINT_ITERATIONS)
	hyperfine -w 5 -r 10 -N \
		-n "revm_erc20_mint" "target/release/benchmark revm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)" \
		-n "levm_erc20_mint" "target/release/benchmark levm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)"
	@echo

revm-comparison-ci: compile-contracts
	$(MAKE) build-revm-comparison
	@echo
	# factorial
	@printf "%s\n" "revm_factorial"
	@target/release/benchmark revm Factorial 1 $(BENCH_FACT_ITERATIONS)
	@printf "%s\n" "levm_factorial"
	@target/release/benchmark levm Factorial 1 $(BENCH_FACT_ITERATIONS)
	hyperfine -w 5 -r 10 -N --export-markdown factorial.md \
		-n "revm_factorial" "target/release/benchmark revm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)" \
		-n "levm_factorial" "target/release/benchmark levm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)"
	@echo
	# fibonacci
	@printf "%s\n" "revm_fibonacci"
	@target/release/benchmark revm Fibonacci 1 $(BENCH_FACT_ITERATIONS)
	@printf "%s\n" "levm_fibonacci"
	@target/release/benchmark levm Fibonacci 1 $(BENCH_FACT_ITERATIONS)
	hyperfine -w 5 -r 10 -N --export-markdown fibonacci.md \
		-n "revm_fibonacci" "target/release/benchmark revm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)" \
		-n "levm_fibonacci" "target/release/benchmark levm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)"
	@echo
	# many_hashes
	@printf "%s\n" "revm_many_hashes"
	target/release/benchmark revm ManyHashes 1 $(BENCH_HASHES_ITERATIONS)
	@printf "%s\n" "levm_many_hashes"
	target/release/benchmark levm ManyHashes 1 $(BENCH_HASHES_ITERATIONS)
	hyperfine -w 5 -r 10 -N --export-markdown many_hashes.md \
		-n "revm_many_hashes" "target/release/benchmark revm ManyHashes $(BENCH_HASHES_REPETITIONS) $(BENCH_HASHES_ITERATIONS)" \
		-n "levm_many_hashes" "target/release/benchmark levm ManyHashes $(BENCH_HASHES_REPETITIONS) $(BENCH_HASHES_ITERATIONS)"
	@echo
	# erc20_approval
	@printf "%s\n" "revm_erc20_approval"
	@target/release/benchmark revm ERC20ApprovalTransfer 1 $(BENCH_APPROVAL_ITERATIONS)
	@printf "%s\n" "levm_erc20_approval"
	@target/release/benchmark levm ERC20ApprovalTransfer 1 $(BENCH_APPROVAL_ITERATIONS)
	hyperfine -w 5 -r 10 -N --export-markdown erc20_approval.md \
		-n "revm_erc20_approval" "target/release/benchmark revm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)" \
		-n "levm_erc20_approval" "target/release/benchmark levm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)"
	@echo
	# erc20_transfer
	@printf "%s\n" "revm_erc20_transfer"
	@target/release/benchmark revm ERC20Transfer 1 $(BENCH_TRANSFER_ITERATIONS)
	@printf "%s\n" "levm_erc20_transfer"
	@target/release/benchmark levm ERC20Transfer 1 $(BENCH_TRANSFER_ITERATIONS)
	hyperfine -w 5 -r 10 -N --export-markdown erc20_transfer.md \
		-n "revm_erc20_transfer" "target/release/benchmark revm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)" \
		-n "levm_erc20_transfer" "target/release/benchmark levm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)"
	@echo
	# erc20_mint
	@printf "%s\n" "revm_erc20_mint"
	@target/release/benchmark revm ERC20Mint 1 $(BENCH_MINT_ITERATIONS)
	@printf "%s\n" "levm_erc20_mint"
	@target/release/benchmark levm ERC20Mint 1 $(BENCH_MINT_ITERATIONS)
	hyperfine -w 5 -r 10 -N --export-markdown erc20_mint.md \
		-n "revm_erc20_mint" "target/release/benchmark revm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)" \
		-n "levm_erc20_mint" "target/release/benchmark levm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)"
	@echo

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

FLAMEGRAPH_DIR := $(EFTEST_DIR)/levm_perfgraphs/flamegraph/bench
flamegraph-benchmarks: ## 🔥 Run benchmarks and create flamegraph
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/flamegraph/bench ||:
    # fibonacci - revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_fibonacci.svg \
	-p revm_comparison --bin benchmark -- revm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)
	# fibonacci - levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_fibonacci.svg \
	-p revm_comparison --bin benchmark -- levm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)
	# factorial - revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_factorial.svg \
	-p revm_comparison --bin benchmark -- revm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)
	# factorial - levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_factorial.svg \
	-p revm_comparison --bin benchmark -- levm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)
	# many_hashes - revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_many_hashes.svg \
	-p revm_comparison --bin benchmark -- revm ManyHashes $(BENCH_HASHES_REPETITIONS) $(BENCH_HASHES_ITERATIONS)
	# many_hashes - levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_many_hashes.svg \
	-p revm_comparison --bin benchmark -- levm ManyHashes $(BENCH_HASHES_REPETITIONS) $(BENCH_HASHES_ITERATIONS)
	# erc20_transfer - revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_erc20_transfer.svg \
	-p revm_comparison --bin benchmark -- revm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)
    # erc20_transfer - levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_erc20_transfer.svg \
	-p revm_comparison --bin benchmark -- levm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)
	# erc20_mint - revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_erc20_mint.svg \
	-p revm_comparison --bin benchmark -- revm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)
	# erc20_mint - levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_erc20_mint.svg \
	-p revm_comparison --bin benchmark -- levm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)
	# erc20_approval - revm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/revm_erc20_approval.svg \
	-p revm_comparison --bin benchmark -- revm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)
	# erc20_approval - levm
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output $(FLAMEGRAPH_DIR)/levm_erc20_approval.svg \
	-p revm_comparison --bin benchmark -- levm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)

SAMPLY_DIR := ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/bench
samply-benchmarks: ## ⚡️ Run benchmarks and create samply profiling file
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/samply/bench ||:
	# fibonacci - revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_revm_fibonacci.json \
	cargo run --release -p revm_comparison --bin benchmark -- revm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)
	# fibonacci - levm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_fibonacci.json \
	cargo run --release -p revm_comparison --bin benchmark -- levm Fibonacci $(BENCH_FIB_REPETITIONS) $(BENCH_FIB_ITERATIONS)
	# factorial - revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_revm_factorial.json \
	cargo run --release -p revm_comparison --bin benchmark -- revm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)
	# factorial - revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_factorial.json \
	cargo run --release -p revm_comparison --bin benchmark -- levm Factorial $(BENCH_FACT_REPETITIONS) $(BENCH_FACT_ITERATIONS)
	# erc20_approval - revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_revm_erc20_approval.json \
	cargo run --release -p revm_comparison --bin benchmark -- revm ERC20ApprovalTransfer $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)
	# erc20_approval - levm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_erc20_approval.json \
	cargo run --release -p revm_comparison --bin benchmark -- levm ERC20ApprovalTransfer  $(BENCH_APPROVAL_REPETITIONS) $(BENCH_APPROVAL_ITERATIONS)
	# erc20_transfer - revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_revm_erc20_transfer.json \
	cargo run --release -p revm_comparison --bin benchmark -- revm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)
	# erc20_transfer - levm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_erc20_transfer.json \
	cargo run --release -p revm_comparison --bin benchmark -- levm ERC20Transfer $(BENCH_TRANSFER_REPETITIONS) $(BENCH_TRANSFER_ITERATIONS)
	# erc20_mint - revm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_revm_erc20_mint.json \
	cargo run --release -p revm_comparison --bin benchmark -- revm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)
	# erc20_mint - levm
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o $(SAMPLY_DIR)/prof_levm_erc20_mint.json \
	cargo run --release -p revm_comparison --bin benchmark -- levm ERC20Mint $(BENCH_MINT_REPETITIONS) $(BENCH_MINT_ITERATIONS)

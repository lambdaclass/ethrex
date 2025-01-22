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
VECTORS_DIR := ../../../cmd/ef_tests/levm/vectors
TMP_DIR := tmp

SPECTEST_VERSION := v14.1
SPECTEST_ARTIFACT := tests_$(SPECTEST_VERSION).tar.gz
SPECTEST_URL := https://github.com/ethereum/tests/archive/refs/tags/$(SPECTEST_VERSION).tar.gz

STATETEST_VERSION := pectra-devnet-5%40v1.1.0
STATETEST_NET := pectra-devnet-5
STATETEST_ARTIFACT := fixtures_$(STATETEST_NET).tar.gz
STATETEST_URL := https://github.com/ethereum/execution-spec-tests/releases/download/$(STATETEST_VERSION)/fixtures_$(STATETEST_NET).tar.gz

download-evm-ef-tests: ## 📥 Download EF Tests
	mkdir -p $(TMP_DIR)
	mkdir -p $(VECTORS_DIR)/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/stEIP2537
	mkdir -p $(VECTORS_DIR)/state_tests
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_URL)
	tar -xzf $(SPECTEST_ARTIFACT) -C $(TMP_DIR)
	mv $(TMP_DIR)/tests-14.1/GeneralStateTests/* $(VECTORS_DIR)/GeneralStateTests/
	mv $(TMP_DIR)/tests-14.1/EIPTests/StateTests/stEIP2537/* $(VECTORS_DIR)/stEIP2537/
	curl -L -o $(STATETEST_ARTIFACT) $(STATETEST_URL)
	tar -xzf $(STATETEST_ARTIFACT) -C $(TMP_DIR)
	mv $(TMP_DIR)/fixtures/state_tests/* $(VECTORS_DIR)/state_tests/
	rm -rf $(TMP_DIR) tests_*.tar.gz
	rm -rf $(TMP_DIR) fixtures_*.tar.gz

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	if [ "$(QUIET)" = "true" ]; then \
		cd ../../../ && \
		time cargo test --quiet -p ef_tests-levm --test ef_tests_levm --release -- $(flags) --summary;\
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

revm-comparison: ## 📊 Run benchmarks of fibonacci and factorial for both REVM and LEVM
	$(MAKE) build-revm-comparison
	@echo
	@printf "%s\n" "revm_factorial"
	@target/release/revm_factorial 1 1000
	@printf "%s\n" "levm_factorial"
	@target/release/levm_factorial 1 1000
	hyperfine -w 5 -r 10 -N \
		-n "revm_factorial" "target/release/revm_factorial 100000 1000" \
		-n "levm_factorial" "target/release/levm_factorial 100000 1000"
	@echo
	@printf "%s\n" "revm_fibonacci"
	@target/release/revm_fibonacci 1 1000
	@printf "%s\n" "levm_fibonacci"
	@target/release/levm_fibonacci 1 1000
	hyperfine -w 5 -r 10 -N \
		-n "revm_fibonacci" "target/release/revm_fibonacci 100000 1000" \
		-n "levm_fibonacci" "target/release/levm_fibonacci 100000 1000"
	@echo
	@printf "%s\n" "revm_snailtracer"
	@target/release/revm_snailtracer 1 1000
	@printf "%s\n" "levm_snailtracer"
	@target/release/levm_snailtracer 1 1000
	hyperfine -w 5 -r 10 -N \
		-n "revm_snailtracer" "target/release/revm_snailtracer 100000 1000" \
		-n "levm_snailtracer" "target/release/levm_snailtracer 100000 1000"
	@echo
	@printf "%s\n" "revm_ten_thousand_hashes"
	@target/release/revm_ten_thousand_hashes 1 1000
	@printf "%s\n" "levm_ten_thousand_hashes"
	@target/release/levm_ten_thousand_hashes 1 1000
	hyperfine -w 5 -r 10 -N \
		-n "revm_ten_thousand_hashes" "target/release/revm_ten_thousand_hashes 100000 1000" \
		-n "levm_ten_thousand_hashes" "target/release/levm_ten_thousand_hashes 100000 1000"
	@echo

revm-comparison-ci:
	$(MAKE) build-revm-comparison
	@echo
	@printf "%s" "revm_factorial result: "
	@target/release/revm_factorial 1 1000
	@printf "%s" "levm_factorial result: "
	@target/release/levm_factorial 1 1000
	hyperfine -w 5 -r 10 -N --export-markdown factorial.md \
		-n "revm_factorial" "target/release/revm_factorial 100000 1000" \
		-n "levm_factorial" "target/release/levm_factorial 100000 1000"
	@echo
	@printf "%s" "revm_fibonacci result: "
	@target/release/revm_fibonacci 1 1000
	@printf "%s" "levm_fibonacci result: "
	@target/release/levm_fibonacci 1 1000
	hyperfine -w 5 -r 10 -N --export-markdown fibonacci.md \
		-n "revm_fibonacci" "target/release/revm_fibonacci 100000 1000" \
		-n "levm_fibonacci" "target/release/levm_fibonacci 100000 1000"
	@echo

build-revm-comparison:
	cd bench/revm_comparison && \
		CARGO_TARGET_DIR=../../target \
		cargo build --release \
		--bin revm_factorial \
		--bin levm_factorial \
		--bin revm_fibonacci \
		--bin levm_fibonacci \
		--bin levm_ten_thousand_hashes \
		--bin revm_ten_thousand_hashes \
		--bin levm_snailtracer \
		--bin revm_snailtracer

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

SUBDIRS := $(shell find $(SPECTEST_VECTORS_DIR)/GeneralStateTests -maxdepth 1 -type d ! -path "$(SPECTEST_VECTORS_DIR)/GeneralStateTests" -exec basename {} \;)

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/levm ||: && \
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/revm ||:
	$(MAKE) flamegraph-run-ef-tests-revm
	$(MAKE) flamegraph-run-ef-tests-levm

flamegraph-run-ef-tests-revm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/ef_tests/revm/$$dir.svg\
		-p ef_tests-levm --test ef_tests_levm -- --summary --revm --tests $$dir;\
	done
flamegraph-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/ef_tests/levm/$$dir.svg\
		-p ef_tests-levm --test ef_tests_levm -- --summary --tests $$dir;\
	done

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	cd ../../../cmd/ef_tests/levm && \
	mkdir -p levm_perfgraphs/samply/ef_tests/levm ||: && \
	mkdir -p levm_perfgraphs/samply/ef_tests/revm ||:
	$(MAKE) samply-run-ef-tests-revm
	$(MAKE) samply-run-ef-tests-levm

samply-run-ef-tests-revm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/ef_tests/revm/prof_$$dir.json \
		cargo test --release -p ef_tests-levm --test ef_tests_levm -- --summary --revm --tests $$dir;\
	done

samply-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/ef_tests/levm/prof_$$dir.json \
		cargo test --release -p ef_tests-levm --test ef_tests_levm -- --summary --tests $$dir;\
	done

flamegraph-benchmarks: ## 🔥 Run benchmarks and create flamegraph
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/flamegraph/bench ||:
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/revm_fibonacci.svg \
	-p revm_comparison --bin revm_fibonacci -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/levm_fibonacci.svg \
	-p revm_comparison --bin levm_fibonacci -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/revm_factorial.svg \
	-p revm_comparison --bin revm_factorial -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/levm_factorial.svg \
	-p revm_comparison --bin levm_factorial -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/levm_snailtracer.svg \
    -p revm_comparison --bin levm_snailtracer -- 100000 1000
    CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
    --root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/revm_snailtracer.svg \
    -p revm_comparison --bin revm_snailtracer -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/levm_ten_thousand_hashes.svg \
	-p revm_comparison --bin levm_ten_thousand_hashes -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
	--root --output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/bench/revm_ten_thousand_hashes.svg \
	-p revm_comparison --bin revm_ten_thousand_hashes -- 100000 1000

samply-benchmarks: ## ⚡️ Run benchmarks and create samply profiling file
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/samply/bench ||:
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/bench/prof_revm_fibonacci.json \
	cargo run --release -p revm_comparison --bin revm_fibonacci -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/bench/prof_levm_fibonacci.json \
	cargo run --release -p revm_comparison --bin levm_fibonacci -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/bench/prof_revm_factorial.json \
	cargo run --release -p revm_comparison --bin revm_factorial -- 100000 1000
	CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
	-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/bench/prof_levm_factorial.json \
	cargo run --release -p revm_comparison --bin levm_factorial -- 100000 1000

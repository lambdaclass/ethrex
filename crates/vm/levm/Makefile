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

SPECTEST_VERSION := v14.1
SPECTEST_ARTIFACT := tests_$(SPECTEST_VERSION).tar.gz
SPECTEST_VECTORS_DIR := ../../../cmd/ef_tests/levm/vectors

$(SPECTEST_ARTIFACT):
	rm -f tests_*.tar.gz # Delete older versions
	curl -L -o $(SPECTEST_ARTIFACT) "https://github.com/ethereum/tests/archive/refs/tags/$(SPECTEST_VERSION).tar.gz"

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	mkdir -p $(SPECTEST_VECTORS_DIR) tmp
	tar -xzf $(SPECTEST_ARTIFACT) -C tmp
	mv tmp/tests-14.1/GeneralStateTests $(SPECTEST_VECTORS_DIR)

download-evm-ef-tests: $(SPECTEST_VECTORS_DIR) ## 📥 Download EF Tests

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	cd ../../../ && \
	time cargo test -p ef_tests-levm --test ef_tests_levm --release -- $(flags)

run-evm-ef-tests-ci: ## 🏃‍♂️ Run EF Tests only with LEVM and without spinner, for CI.
	cd ../../../ && \
	time cargo test -p ef_tests-levm --test ef_tests_levm --release -- --summary

generate-evm-ef-tests-report: ## 📊 Generate EF Tests Report
	cd ../../../ && \
	cargo test -p ef_tests-levm --test ef_tests_levm --release -- --summary

clean-evm-ef-tests: ## 🗑️  Clean test vectors
	rm -rf $(SPECTEST_VECTORS_DIR)

###### Benchmarks ######

revm-comparison:
	$(MAKE) build-revm-comparison
	@echo
	@printf "%s" "revm_factorial result: "
	@target/release/revm_factorial 1 1000
	@printf "%s" "levm_factorial result: "
	@target/release/levm_factorial 1 1000
	hyperfine -w 5 -r 10 -N \
		-n "revm_factorial" "target/release/revm_factorial 100000 1000" \
		-n "levm_factorial" "target/release/levm_factorial 100000 1000"
	@echo
	@printf "%s" "revm_fibonacci result: "
	@target/release/revm_fibonacci 1 1000
	@printf "%s" "levm_fibonacci result: "
	@target/release/levm_fibonacci 1 1000
	hyperfine -w 5 -r 10 -N \
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
		--bin levm_fibonacci

###### Build Client with LEVM ######

FLAGS := "--features ethrex-blockchain/levm,ethrex-vm/levm,ethrex/levm"

build-image-levm: ## 🐳 Build the Docker image with LEVM features
	cd ../../../ && \
	docker build -t ethrex --build-arg BUILD_FLAGS=$(FLAGS) .

run-hive-debug-levm: build-image-levm ## 🐝 Run Hive with LEVM in debug mode
	$(MAKE) -C ../../../ setup-hive
	cd ../../../hive && ./hive --sim $(SIMULATION) --client ethrex --sim.limit "$(TEST_PATTERN)" --docker.output

SUBDIRS := $(shell find $(SPECTEST_VECTORS_DIR)/GeneralStateTests -maxdepth 1 -type d ! -path "$(SPECTEST_VECTORS_DIR)/GeneralStateTests" -exec basename {} \;)

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/flamegraph ||:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output ../../../cmd/ef_tests/levm/levm_perfgraphs/flamegraph/$$dir.svg\
		-p ef_tests-levm --test ef_tests_levm -- --summary --tests $$dir;\
	done

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	cd ../../../cmd/ef_tests/levm &&\
	mkdir -p levm_perfgraphs/samply ||:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o ../../../cmd/ef_tests/levm/levm_perfgraphs/samply/prof_$$dir.json \
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

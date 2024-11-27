.PHONY: all test clippy fmt usage lint eth-tests

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
	time cargo test -p ef_tests-levm --test ef_tests_levm

generate-evm-ef-tests-report: ## 📊 Generate EF Tests Report
	cd ../../../ && \
	cargo test -p ef_tests-levm --test ef_tests_levm -- --summary

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

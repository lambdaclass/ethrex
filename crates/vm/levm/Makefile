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
download-evm-ef-tests: ## 📥 Download EF Tests
	cd ../../../tooling/ef_tests/state && \
	make download-evm-ef-tests flags="$(flags)"

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	cd ../../../tooling/ef_tests/state && \
	make run-evm-ef-tests flags="$(flags)"

###### Build Client with LEVM ######
EFTEST_DIR := ../../../tooling/ef_tests/state
VECTORS_DIR := $(EFTEST_DIR)/vectors

download-state-tests: ## 📥 Download and setup state tests fixtures
	cd $(EFTEST_DIR) &&\
	$(MAKE) download-evm-ef-tests

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	cd $(EFTEST_DIR) &&\
	$(MAKE) flamegraph-run-ef-tests

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	cd $(EFTEST_DIR) && \
	$(MAKE) samply-run-ef-tests

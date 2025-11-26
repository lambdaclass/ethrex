.PHONY: help download-evm-ef-tests clean-evm-ef-tests refresh-evm-ef-tests run-new-runner clean-reports

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

FIXTURES_FILE := .fixtures_url
STATETEST_ARTIFACT := test.tar.gz
VECTORS_DIR := vectors

TMP_DIR := tmp
TESTS_REPO := $(TMP_DIR)/ethereum-tests

ETH_TEST_URL := https://github.com/ethereum/tests.git
ETH_TEST_TAG := v17.0
COMMIT_LEGACY_TESTS_FOR_TAG := b3f67fe

STATETEST_URL := $(shell cat $(FIXTURES_FILE))

$(STATETEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-evm-ef-tests
	curl -L -o $(STATETEST_ARTIFACT) $(STATETEST_URL)

$(VECTORS_DIR): $(STATETEST_ARTIFACT)
	$(MAKE) setup-test-dirs
	$(MAKE) clone-ef-tests
	tar -xzf $(STATETEST_ARTIFACT) --strip-components=2 -C $(VECTORS_DIR)/state_tests fixtures/state_tests
	rm -rf $(TMP_DIR)
	@# Keep vectors newer than the downloaded archive so we skip re-downloading when the URL is unchanged
	@touch $(VECTORS_DIR)

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
	cp -r $(TESTS_REPO)/LegacyTests/Cancun/GeneralStateTests/* $(VECTORS_DIR)/LegacyTests/Cancun/GeneralStateTests/

download-evm-ef-tests: $(VECTORS_DIR) ## 📥 Download and setup state tests fixtures

clean-evm-ef-tests: ## 🗑️ Clean test vectors and temporary files
	rm -rf $(VECTORS_DIR)
	rm -rf $(TMP_DIR)
	rm -f $(STATETEST_ARTIFACT)

refresh-evm-ef-tests: clean-evm-ef-tests download-evm-ef-tests ## Cleans and re-downloads tests, useful when they are outdated!

CARGO_TARGET_DIR ?= ./target
run-new-runner: $(VECTORS_DIR) ## 🧪 Run all tests with optional flags. Usage: make run-new-runner flags="--flag1 --flag2"
	CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo run --package ef_tests-statev2 --release -- $(flags)

clean-reports: ## 🗑️  Delete all generated test reports
	rm -rf ./reports

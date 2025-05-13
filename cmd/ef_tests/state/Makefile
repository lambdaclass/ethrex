.PHONY: run-evm-ef-tests

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
	rm -f $(STATETEST_ARTIFACT)
	rm -rf $(TMP_DIR)

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

setup-test-dirs:
	mkdir -p $(VECTORS_DIR)
	mkdir -p $(VECTORS_DIR)/LegacyTests/Constantinople/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/LegacyTests/Cancun/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/GeneralStateTests
	mkdir -p $(VECTORS_DIR)/state_tests

clone-ef-tests: ## 📥 Download Ethereum Tests repository with submodules
	mkdir -p $(TMP_DIR)
	git clone --recurse-submodules --depth 1 --branch $(ETH_TEST_TAG) $(ETH_TEST_URL) $(TESTS_REPO)
	cd $(TESTS_REPO)/LegacyTests && git checkout $(COMMIT_LEGACY_TESTS_FOR_TAG)
	cp -r $(TESTS_REPO)/GeneralStateTests/* $(VECTORS_DIR)/GeneralStateTests/
	cp -r $(TESTS_REPO)/LegacyTests/Constantinople/GeneralStateTests/* $(VECTORS_DIR)/LegacyTests/Constantinople/GeneralStateTests/
	cp -r $(TESTS_REPO)/LegacyTests/Cancun/GeneralStateTests/* $(VECTORS_DIR)/LegacyTests/Cancun/GeneralStateTests/;

download-state-tests: $(VECTORS_DIR) ## 📥 Download and setup state tests fixtures

clean-evm-ef-tests: ## 🗑️ Clean test vectors and temporary files
	rm -rf $(VECTORS_DIR)
	rm -rf $(TMP_DIR)
	rm -f $(STATETEST_ARTIFACT)

refresh-evm-ef-tests: clean-evm-ef-tests download-evm-ef-tests ## Cleans and re-downloads tests, useful when they are outdated!

run-evm-ef-tests: $(VECTORS_DIR) ## 🏃‍♂️ Run EF Tests
	if [ "$(QUIET)" = "true" ]; then \
		time cargo test --quiet --test all --release -- $(flags) --summary;\
	elif [ "$(DEBUG)" = "true" ]; then \
		time cargo test --test all -- $(flags);\
	else \
		time cargo test --test all --release -- $(flags);\
	fi

run-evm-ef-tests-ci: $(VECTORS_DIR) ## 🏃‍♂️ Run EF Tests only with LEVM and without spinner, for CI.
	time cargo test --test all --release -- --summary

generate-evm-ef-tests-report:$(VECTORS_DIR) ## 📊 Generate EF Tests Report
	cargo test --test all --release -- --summary

test-levm: $(VECTORS_DIR)
	$(MAKE) run-evm-ef-tests flags="--summary"

test-revm: $(VECTORS_DIR)
	$(MAKE) run-evm-ef-tests flags="--revm"

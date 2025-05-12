.PHONY: run-evm-ef-tests

VECTORS_DIR := vectors
TMP_DIR := tmp
TESTS_REPO := $(TMP_DIR)/ethereum-tests

ETH_TEST_URL := https://github.com/ethereum/tests.git
ETH_TEST_TAG := v17.0
COMMIT_LEGACY_TESTS_FOR_TAG := b3f67fe

STATETEST_VERSION := pectra-devnet-6%40v1.0.0
STATETEST_NET := pectra-devnet-6
STATETEST_ARTIFACT := fixtures_$(STATETEST_NET).tar.gz
STATETEST_URL := https://github.com/ethereum/execution-spec-tests/releases/download/$(STATETEST_VERSION)/fixtures_$(STATETEST_NET).tar.gz

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

refresh-evm-ef-tests: clean-evm-ef-tests download-evm-ef-tests ## Cleans and re-downloads tests, useful when they are outdated!

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	if [ "$(QUIET)" = "true" ]; then \
		cd ../../../ && \
		time cargo test --quiet -p ef_tests-state --test all --release -- $(flags) --summary;\
	elif [ "$(DEBUG)" = "true" ]; then \
		cd ../../../ && \
		time cargo test -p ef_tests-state --test all -- $(flags);\
	else \
		cd ../../../ && \
		time cargo test -p ef_tests-state --test all --release -- $(flags);\
	fi

run-evm-ef-tests-ci: ## 🏃‍♂️ Run EF Tests only with LEVM and without spinner, for CI.
	cd ../../../ && \
	time cargo test -p ef_tests-state --test all --release -- --summary

generate-evm-ef-tests-report: ## 📊 Generate EF Tests Report
	cd ../../../ && \
	cargo test -p ef_tests-state --test all --release -- --summary

test-levm: 
	$(MAKE) run-evm-ef-tests flags="--summary"

test-revm: 
	$(MAKE) run-evm-ef-tests flags="--revm"

.PHONY: download-evm-ef-tests clean-evm-ef-tests run-evm-ef-tests test-levm test-levm-nostd-crypto run-evm-ef-tests flamegraph-run-ef-tests samply-run-ef-tests amsterdam-vectors

FIXTURES_FILE := ../.fixtures_url
STATETEST_ARTIFACT := test.tar.gz
VECTORS_DIR := vectors

TMP_DIR := tmp
TESTS_REPO := $(TMP_DIR)/ethereum-tests

ETH_TEST_URL := https://github.com/ethereum/tests.git
ETH_TEST_TAG := v17.0
COMMIT_LEGACY_TESTS_FOR_TAG := b3f67fe

STATETEST_URL := $(shell cat $(FIXTURES_FILE))

AMSTERDAM_FIXTURES_FILE := ../.fixtures_url_amsterdam
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

$(VECTORS_DIR)/state_tests/for_amsterdam: $(AMSTERDAM_ARTIFACT) $(VECTORS_DIR)
	tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(VECTORS_DIR)/state_tests fixtures/state_tests/for_amsterdam

amsterdam-vectors: $(VECTORS_DIR)/state_tests/for_amsterdam

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
		time cargo test --quiet --test all --profile release-fast -- $(flags) --summary;\
	elif [ "$(DEBUG)" = "true" ]; then \
		time cargo test --test all -- $(flags);\
	else \
		time cargo test --test all --profile release-fast -- $(flags);\
	fi

run-evm-ef-tests-ci: $(VECTORS_DIR) ## 🏃‍♂️ Run EF Tests only with LEVM and without spinner, for CI.
	time cargo test -p ef_tests-state --test all --profile release-fast -- --summary

test-levm: $(VECTORS_DIR)
	$(MAKE) run-evm-ef-tests flags="--summary"

test-levm-nostd-crypto: $(VECTORS_DIR) ## 🧪 Run EF state tests with no_std crypto fallbacks
	cargo test --test all --profile release-fast --no-default-features --features nostd-crypto -- --summary

###### Running Flamegraphs ######

SUBDIRS := $(shell find $(VECTORS_DIR)/GeneralStateTests -maxdepth 1 -type d ! -path "$(VECTORS_DIR)/GeneralStateTests" -exec basename {} \;)

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	mkdir -p levm_perfgraphs/flamegraph/ef_tests/state ||:
	$(MAKE) flamegraph-run-ef-tests-levm

flamegraph-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root \
		--output levm_perfgraphs/flamegraph/ef_tests/state/$$dir.svg\
		-p ef_tests-state --test all -- --summary --tests $$dir;\
	done

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	mkdir -p levm_perfgraphs/samply/ef_tests/state ||:
	$(MAKE) samply-run-ef-tests-levm

samply-run-ef-tests-levm:
	@for dir in $(SUBDIRS); do\
		CARGO_PROFILE_RELEASE_DEBUG=true samply record --save-only \
		-o levm_perfgraphs/samply/ef_tests/state/prof_$$dir.json \
		cargo test --profile release-fast -p ef_tests-state --test all -- --summary --tests $$dir;\
	done

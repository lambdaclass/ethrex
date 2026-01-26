.PHONY: download-test-vectors generate-test-vectors clean-vectors clean-execution-specs test test-levm test-sp1 test-stateless

VECTORS_ROOT := vectors
SPECTEST_FIXTURES_DIR := fixtures
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest

# execution-specs configuration
EXECUTION_SPECS_DIR := execution-specs
EXECUTION_SPECS_REPO := https://github.com/ethereum/execution-specs.git
EXECUTION_SPECS_REF := forks/amsterdam

# Fill command options
FILL_PROCESSES := 10
FILL_MARKERS := not slow and not zkevm and not benchmark

# Legacy tests configuration
LEGACYTEST_ARTIFACT := legacy-tests.tar.gz
LEGACYTEST_VECTORS_DIR := $(VECTORS_ROOT)/legacy
LEGACYTEST_COMMIT := 1f581b8ccdc4c63acf5f2c5c1b155c690c32a8eb
LEGACYTEST_URL := https://github.com/ethereum/legacytests/archive/$(LEGACYTEST_COMMIT).tar.gz
LEGACYTEST_ARCHIVE_ROOT := legacytests-$(LEGACYTEST_COMMIT)/Cancun/BlockchainTests

VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR) $(LEGACYTEST_VECTORS_DIR)

# Clone execution-specs repository
$(EXECUTION_SPECS_DIR):
	@command -v uv >/dev/null 2>&1 || { echo "Error: uv is required but not installed. Install from https://docs.astral.sh/uv/"; exit 1; }
	git clone --depth 1 --branch $(EXECUTION_SPECS_REF) $(EXECUTION_SPECS_REPO) $(EXECUTION_SPECS_DIR)

# Generate fixtures using fill command
$(SPECTEST_FIXTURES_DIR): $(EXECUTION_SPECS_DIR)
	cd $(EXECUTION_SPECS_DIR) && uv run fill \
		-m "$(FILL_MARKERS)" \
		-n auto --maxprocesses $(FILL_PROCESSES) --dist=loadgroup \
		--until Amsterdam \
		--clean \
		--output $(CURDIR)/$(SPECTEST_FIXTURES_DIR) \
		tests

$(SPECTEST_VECTORS_DIR): $(SPECTEST_FIXTURES_DIR)
	rm -rf $(SPECTEST_VECTORS_DIR)
	mkdir -p $(SPECTEST_VECTORS_DIR)
	cp -r $(SPECTEST_FIXTURES_DIR)/blockchain_tests/* $(SPECTEST_VECTORS_DIR)/

$(LEGACYTEST_ARTIFACT):
	curl -L -o $(LEGACYTEST_ARTIFACT) $(LEGACYTEST_URL)

$(LEGACYTEST_VECTORS_DIR): $(LEGACYTEST_ARTIFACT)
	rm -rf $(LEGACYTEST_VECTORS_DIR)
	mkdir -p $(LEGACYTEST_VECTORS_DIR)
	tar -xzf $(LEGACYTEST_ARTIFACT) --strip-components=3 -C $(LEGACYTEST_VECTORS_DIR) \
		$(LEGACYTEST_ARCHIVE_ROOT)/GeneralStateTests \
		$(LEGACYTEST_ARCHIVE_ROOT)/ValidBlocks

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) ## 📥 Generate and extract test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -rf $(SPECTEST_FIXTURES_DIR)
	rm -f $(LEGACYTEST_ARTIFACT)

clean-execution-specs: ## 🗑️  Clean cloned execution-specs repository
	rm -rf $(EXECUTION_SPECS_DIR)

test-levm: $(VECTORS_TARGETS) ## 🧪 Run blockchain tests with LEVM
	cargo test --profile release-with-debug

test-sp1: $(VECTORS_TARGETS)
	cargo test --profile release-with-debug --features sp1

test-stateless: $(VECTORS_TARGETS)
	cargo test --profile release-with-debug --features stateless

test: ## 🧪 Run blockchain tests with LEVM both with state and stateless
	$(MAKE) test-levm
	$(MAKE) test-stateless

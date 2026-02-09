.PHONY: download-test-vectors clean-vectors test test-levm test-sp1 test-stateless amsterdam-vectors

VECTORS_ROOT := vectors
FIXTURES_FILE := .fixtures_url
SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

LEGACYTEST_ARTIFACT := legacy-tests.tar.gz
LEGACYTEST_VECTORS_DIR := $(VECTORS_ROOT)/legacy
LEGACYTEST_COMMIT := 1f581b8ccdc4c63acf5f2c5c1b155c690c32a8eb
LEGACYTEST_URL := https://github.com/ethereum/legacytests/archive/$(LEGACYTEST_COMMIT).tar.gz
LEGACYTEST_ARCHIVE_ROOT := legacytests-$(LEGACYTEST_COMMIT)/Cancun/BlockchainTests

AMSTERDAM_FIXTURES_FILE := .fixtures_url_amsterdam
AMSTERDAM_ARTIFACT := amsterdam-tests.tar.gz
AMSTERDAM_URL := $(shell cat $(AMSTERDAM_FIXTURES_FILE))

VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR) $(LEGACYTEST_VECTORS_DIR)

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_URL)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	rm -rf $(SPECTEST_VECTORS_DIR)
	mkdir -p $(SPECTEST_VECTORS_DIR)
	tar -xzf $(SPECTEST_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests

$(LEGACYTEST_ARTIFACT):
	curl -L -o $(LEGACYTEST_ARTIFACT) $(LEGACYTEST_URL)

$(LEGACYTEST_VECTORS_DIR): $(LEGACYTEST_ARTIFACT)
	rm -rf $(LEGACYTEST_VECTORS_DIR)
	mkdir -p $(LEGACYTEST_VECTORS_DIR)
	tar -xzf $(LEGACYTEST_ARTIFACT) --strip-components=3 -C $(LEGACYTEST_VECTORS_DIR) \
		$(LEGACYTEST_ARCHIVE_ROOT)/GeneralStateTests \
		$(LEGACYTEST_ARCHIVE_ROOT)/ValidBlocks

$(AMSTERDAM_ARTIFACT): $(AMSTERDAM_FIXTURES_FILE)
	curl -L -o $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_URL)

amsterdam-vectors: $(AMSTERDAM_ARTIFACT) $(SPECTEST_VECTORS_DIR)
	tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests/amsterdam

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(LEGACYTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT)

test-levm: $(VECTORS_TARGETS) amsterdam-vectors ## 🧪 Run blockchain tests with LEVM
	cargo test --profile release-with-debug

test-sp1: $(VECTORS_TARGETS) amsterdam-vectors
	cargo test --profile release-with-debug --features sp1

test-stateless: $(VECTORS_TARGETS) amsterdam-vectors
	cargo test --profile release-with-debug --features stateless

test: ## 🧪 Run blockchain tests with LEVM both with state and stateless 
	$(MAKE) test-levm
	$(MAKE) test-stateless

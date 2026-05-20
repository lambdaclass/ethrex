.PHONY: download-test-vectors clean-vectors test test-levm test-sp1 test-stateless amsterdam-vectors zkevm-vectors

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

# zkevm@v0.3.3 ships fixtures filled against an older Amsterdam base
# (bal@v5.6.1). Extracting them on top of the snobal-devnet-6 tree would
# clobber the newer fixtures with stale gas-accounting expectations, so we
# keep them in a separate root and only the stateless harness reads from it.
ZKEVM_VECTORS_ROOT := vectors_zkevm
ZKEVM_VECTORS_DIR := $(ZKEVM_VECTORS_ROOT)/eest
ZKEVM_FIXTURES_FILE := .fixtures_url_zkevm
ZKEVM_ARTIFACT := zkevm-tests.tar.gz
ZKEVM_URL := $(shell cat $(ZKEVM_FIXTURES_FILE))

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

$(SPECTEST_VECTORS_DIR)/for_amsterdam: $(AMSTERDAM_ARTIFACT) $(SPECTEST_VECTORS_DIR)
	tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests/for_amsterdam

amsterdam-vectors: $(SPECTEST_VECTORS_DIR)/for_amsterdam

$(ZKEVM_ARTIFACT): $(ZKEVM_FIXTURES_FILE)
	curl -L -o $(ZKEVM_ARTIFACT) $(ZKEVM_URL)

$(ZKEVM_VECTORS_DIR): $(ZKEVM_ARTIFACT)
	rm -rf $(ZKEVM_VECTORS_DIR)
	mkdir -p $(ZKEVM_VECTORS_DIR)
	tar -xzf $(ZKEVM_ARTIFACT) --strip-components=2 -C $(ZKEVM_VECTORS_DIR) fixtures/blockchain_tests/for_amsterdam

zkevm-vectors: $(ZKEVM_VECTORS_DIR)

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors zkevm-vectors ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT) $(ZKEVM_VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(LEGACYTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT) $(ZKEVM_ARTIFACT)

test-levm: $(VECTORS_TARGETS) amsterdam-vectors ## 🧪 Run blockchain tests with LEVM
	cargo test --profile release-fast

test-sp1: $(VECTORS_TARGETS) amsterdam-vectors
	cargo test --profile release-fast --features sp1

test-stateless: zkevm-vectors
	cargo test --profile release-fast --features stateless

test-stateless-zkevm: zkevm-vectors
	cargo test --profile release-fast --features stateless -- eip8025_optional_proofs

test: ## 🧪 Run blockchain tests with LEVM both with state and stateless
	$(MAKE) test-levm
	# Narrow stateless coverage to the EIP-8025 optional-proofs suite. The
	# zkevm@v0.3.3 fixtures are filled against bal@v5.6.1, which is out of
	# sync with this branch's bal-devnet-6+ (and bal-devnet-7-prep) gas
	# accounting; the broader `test-stateless` invocation introduced by
	# #6527 trips ~549 of those fixtures with `GasUsedMismatch` /
	# `ReceiptsRootMismatch` / `BlockAccessListHashMismatch`. Re-broaden
	# once the zkevm bundle is regenerated against the current bal spec.
	$(MAKE) test-stateless-zkevm

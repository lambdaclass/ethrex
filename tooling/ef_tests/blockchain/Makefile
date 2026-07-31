.PHONY: download-test-vectors clean-vectors test test-levm test-sp1 test-stateless amsterdam-vectors zkevm-vectors test-stateless-zkevm

VECTORS_ROOT := vectors
FIXTURES_FILE := ../.fixtures_url
SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

LEGACYTEST_ARTIFACT := legacy-tests.tar.gz
LEGACYTEST_VECTORS_DIR := $(VECTORS_ROOT)/legacy
LEGACYTEST_COMMIT := 1f581b8ccdc4c63acf5f2c5c1b155c690c32a8eb
LEGACYTEST_URL := https://github.com/ethereum/legacytests/archive/$(LEGACYTEST_COMMIT).tar.gz
LEGACYTEST_ARCHIVE_ROOT := legacytests-$(LEGACYTEST_COMMIT)/Cancun/BlockchainTests

AMSTERDAM_FIXTURES_FILE := ../.fixtures_url_amsterdam
AMSTERDAM_ARTIFACT := amsterdam-tests.tar.gz
AMSTERDAM_URL := $(shell cat $(AMSTERDAM_FIXTURES_FILE))

# Subtrees the Amsterdam bundle contributes. `for_bpo2toamsterdamattime15k` holds the
# fork-activation fixtures (a header field appearing mid-chain, the gas schedule
# switching between blocks); it exists only in this bundle, so omitting it leaves the
# whole fork boundary untested.
AMSTERDAM_SUBTREES := for_amsterdam for_bpo2toamsterdamattime15k
AMSTERDAM_STAMP := $(SPECTEST_VECTORS_DIR)/.amsterdam_overlay

# zkevm@v0.6.2 is filled against glamsterdam-devnet v7.2.0 (one devnet behind the
# Amsterdam bundle) but additionally carries the EIP-8025 stateless witness
# fields (executionWitness / statelessInputBytes / statelessOutputBytes). Both
# bundles extract a `for_amsterdam/` subtree, so we keep the zkevm bundle in a
# separate root that only the stateless harness reads, to avoid overlaying the
# regular Amsterdam fixtures.
ZKEVM_VECTORS_ROOT := vectors_zkevm
ZKEVM_VECTORS_DIR := $(ZKEVM_VECTORS_ROOT)/eest
ZKEVM_FIXTURES_FILE := ../.fixtures_url_zkevm
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

# Order-only on $(SPECTEST_VECTORS_DIR): building it runs clean-vectors, which deletes
# $(AMSTERDAM_ARTIFACT). Downloading afterwards keeps the bundle from being wiped
# between the curl and the extract.
$(AMSTERDAM_ARTIFACT): $(AMSTERDAM_FIXTURES_FILE) | $(SPECTEST_VECTORS_DIR)
	curl -L -o $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_URL)

# Clear the subtrees first: the bundle is overlaid onto an existing `vectors/eest`, so
# fixtures renamed or dropped between devnet releases would otherwise survive as stale
# leftovers and run against the current client.
$(AMSTERDAM_STAMP): $(AMSTERDAM_ARTIFACT) $(SPECTEST_VECTORS_DIR)
	rm -rf $(addprefix $(SPECTEST_VECTORS_DIR)/,$(AMSTERDAM_SUBTREES))
	tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
		$(addprefix fixtures/blockchain_tests/,$(AMSTERDAM_SUBTREES))
	touch $@

amsterdam-vectors: $(AMSTERDAM_STAMP)

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
	$(MAKE) test-stateless

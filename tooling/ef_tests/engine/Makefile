.PHONY: download-test-vectors clean-vectors test test-rocksdb test-strict amsterdam-vectors focil-vectors help

VECTORS_ROOT := vectors
FIXTURES_FILE := .fixtures_url
SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

AMSTERDAM_FIXTURES_FILE := .fixtures_url_amsterdam
AMSTERDAM_ARTIFACT := amsterdam-tests.tar.gz
AMSTERDAM_URL := $(shell cat $(AMSTERDAM_FIXTURES_FILE))

# EIP-7805 (FOCIL) engine fixtures. The tarball nests them under the same
# fixtures/blockchain_tests_engine/for_amsterdam/ prefix as the bal release,
# so they overlay into the existing for_amsterdam/ tree.
FOCIL_FIXTURES_FILE := .fixtures_url_focil
FOCIL_ARTIFACT := focil-tests.tar.gz
FOCIL_URL := $(shell cat $(FOCIL_FIXTURES_FILE))
FOCIL_VECTORS_DIR := $(SPECTEST_VECTORS_DIR)/for_amsterdam/bogota/eip7805_focil

VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR)

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_URL)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	rm -rf $(SPECTEST_VECTORS_DIR)
	mkdir -p $(SPECTEST_VECTORS_DIR)
	tar -xzf $(SPECTEST_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests_engine

$(AMSTERDAM_ARTIFACT): $(AMSTERDAM_FIXTURES_FILE)
	curl -L -o $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_URL)

$(SPECTEST_VECTORS_DIR)/for_amsterdam: $(AMSTERDAM_ARTIFACT) $(SPECTEST_VECTORS_DIR)
	tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests_engine/for_amsterdam

amsterdam-vectors: $(SPECTEST_VECTORS_DIR)/for_amsterdam ## 📥 Overlay glamsterdam-devnet-6 engine fixtures onto vectors/eest

$(FOCIL_ARTIFACT): $(FOCIL_FIXTURES_FILE)
	curl -L -o $(FOCIL_ARTIFACT) $(FOCIL_URL)

$(FOCIL_VECTORS_DIR): $(FOCIL_ARTIFACT) $(SPECTEST_VECTORS_DIR)
	tar -xzf $(FOCIL_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests_engine/for_amsterdam/bogota/eip7805_focil

focil-vectors: $(FOCIL_VECTORS_DIR) ## 📥 Overlay EIP-7805 FOCIL engine fixtures onto vectors/eest

help: ## 📚 Show help for each Makefile target
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 📥 Download engine-format test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT) $(FOCIL_ARTIFACT)

test: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 🧪 Run engine-format fixture suite (InMemory, release)
	cargo test -p ef_tests-engine --release

test-rocksdb: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 🧪 Run with RocksDB backend
	cargo test -p ef_tests-engine --release --features rocksdb

test-strict: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 🧪 Run with strict validationError checking
	ETHREX_ENGINE_STRICT_EXCEPTIONS=1 cargo test -p ef_tests-engine --release

.PHONY: download-test-vectors clean-vectors test

FIXTURES_FILE := .fixtures_url
SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := vectors

SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_URL)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	mkdir -p $(SPECTEST_VECTORS_DIR) tmp
	tar -xzf $(SPECTEST_ARTIFACT) -C tmp
	rm -rf $(SPECTEST_VECTORS_DIR)/*
	mv tmp/fixtures/blockchain_tests/* $(SPECTEST_VECTORS_DIR)

download-test-vectors: $(SPECTEST_VECTORS_DIR) ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(SPECTEST_VECTORS_DIR)
	rm -f $(SPECTEST_ARTIFACT)

test: $(SPECTEST_VECTORS_DIR) ## 🧪 Run blockchain tests
	# cargo test --workspace parse_and_execute_until_cancun::eip4844_blobs/blob_txs/invalid_normal_gas.json
	cargo test --workspace -- --test-threads=1

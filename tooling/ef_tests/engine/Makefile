.PHONY: download-test-vectors clean-vectors test test-rocksdb test-strict amsterdam-vectors frames-vectors help

VECTORS_ROOT := vectors
FIXTURES_FILE := ../.fixtures_url
SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

AMSTERDAM_FIXTURES_FILE := ../.fixtures_url_amsterdam
AMSTERDAM_ARTIFACT := amsterdam-tests.tar.gz
AMSTERDAM_URL := $(shell cat $(AMSTERDAM_FIXTURES_FILE))

FRAMES_FIXTURES_FILE := ../.fixtures_url_frames
FRAMES_ARTIFACT := frames-tests.tar.gz
FRAMES_URL := $(shell cat $(FRAMES_FIXTURES_FILE))
# All Bogota-fork fixtures -- the EIP-8141 suites and the converted legacy
# variants alike -- live under this one subtree, so it is the only thing the
# frames bundle contributes here.
FRAMES_SUBTREE := for_bogota
FRAMES_STAMP := $(SPECTEST_VECTORS_DIR)/.frames_overlay

# Subtrees the Amsterdam bundle contributes. `for_bpo2toamsterdamattime15k` holds the
# fork-activation fixtures (a header field appearing mid-chain, the gas schedule
# switching between blocks); it exists only in this bundle, so omitting it leaves the
# whole fork boundary untested.
AMSTERDAM_SUBTREES := for_amsterdam for_bpo2toamsterdamattime15k
AMSTERDAM_STAMP := $(SPECTEST_VECTORS_DIR)/.amsterdam_overlay

VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR)

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	curl -L -o $(SPECTEST_ARTIFACT) $(SPECTEST_URL)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	rm -rf $(SPECTEST_VECTORS_DIR)
	mkdir -p $(SPECTEST_VECTORS_DIR)
	tar -xzf $(SPECTEST_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests_engine

# The overlay is keyed on the release URL it was built from, recorded in
# $(AMSTERDAM_STAMP). Timestamps cannot express this: a bundle left on disk by
# another branch says nothing in its mtime about which release it came from, so one
# that merely looks newer than the URL file would be re-extracted as if it were the
# requested release. Clearing the subtrees before extracting also drops fixtures that
# upstream renamed or removed, which would otherwise survive and run against the
# current client. The fetch lands in a temporary file so a failed download leaves
# the previous bundle and stamp untouched instead of truncating them.
amsterdam-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Overlay Amsterdam fixtures, re-fetching when the pinned release changed
	@have=$$(cat $(AMSTERDAM_STAMP) 2>/dev/null); \
	for subtree in $(AMSTERDAM_SUBTREES); do \
		[ -d $(SPECTEST_VECTORS_DIR)/$$subtree ] || have=; \
	done; \
	if [ "$$have" = "$(AMSTERDAM_URL)" ]; then \
		echo "Amsterdam fixtures already overlaid from $(AMSTERDAM_URL)"; \
	else \
		curl -fL -o $(AMSTERDAM_ARTIFACT).part $(AMSTERDAM_URL) && \
		mv $(AMSTERDAM_ARTIFACT).part $(AMSTERDAM_ARTIFACT) && \
		rm -rf $(addprefix $(SPECTEST_VECTORS_DIR)/,$(AMSTERDAM_SUBTREES)) && \
		tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
			$(addprefix fixtures/blockchain_tests_engine/,$(AMSTERDAM_SUBTREES)) && \
		echo "$(AMSTERDAM_URL)" > $(AMSTERDAM_STAMP); \
	fi

help: ## 📚 Show help for each Makefile target
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

# Keyed on the release URL, exactly like the Amsterdam overlay above and the
# blockchain runner's frames overlay: a subtree left on disk says nothing in its
# mtime about which release produced it, and clearing it first drops fixtures a
# later release renamed or removed.
frames-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Overlay the frames-devnet engine fixtures, re-fetching when the pinned release changed
	@have=$$(cat $(FRAMES_STAMP) 2>/dev/null); \
	[ -d $(SPECTEST_VECTORS_DIR)/$(FRAMES_SUBTREE) ] || have=; \
	if [ "$$have" = "$(FRAMES_URL)" ]; then \
		echo "Frames fixtures already overlaid from $(FRAMES_URL)"; \
	else \
		curl -fL -o $(FRAMES_ARTIFACT).part $(FRAMES_URL) && \
		mv $(FRAMES_ARTIFACT).part $(FRAMES_ARTIFACT) && \
		rm -rf $(SPECTEST_VECTORS_DIR)/$(FRAMES_SUBTREE) && \
		tar -xzf $(FRAMES_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
			fixtures/blockchain_tests_engine/$(FRAMES_SUBTREE) && \
		echo "$(FRAMES_URL)" > $(FRAMES_STAMP); \
	fi

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors ## 📥 Download engine-format test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_ARTIFACT).part $(FRAMES_ARTIFACT) $(FRAMES_ARTIFACT).part

test: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors ## 🧪 Run engine-format fixture suite (InMemory, release)
	cargo test -p ef_tests-engine --release

test-rocksdb: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors ## 🧪 Run with RocksDB backend
	cargo test -p ef_tests-engine --release --features rocksdb

test-strict: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors ## 🧪 Run with strict validationError checking
	ETHREX_ENGINE_STRICT_EXCEPTIONS=1 cargo test -p ef_tests-engine --release

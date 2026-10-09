.PHONY: download-test-vectors clean-vectors test test-rocksdb test-strict frames-vectors help

VECTORS_ROOT := vectors
FIXTURES_FILE := ../.fixtures_url

# Retrying downloader: release downloads fail transiently often enough to be the
# top cause of red CI. See the script header for what each guard catches.
DOWNLOAD := ../../../.github/scripts/download.sh

SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

FRAMES_FIXTURES_FILE := ../.fixtures_url_frames
FRAMES_ARTIFACT := frames-tests.tar.gz
FRAMES_URL := $(shell cat $(FRAMES_FIXTURES_FILE))
# All Bogota-fork fixtures -- the frames suites and the converted legacy variants
# alike -- live under `for_bogota`, and the Amsterdam->Bogota activation fixtures
# under `for_amsterdamtobogotaattime15k`; those two are what the frames bundle
# contributes here.
FRAMES_SUBTREES := for_bogota for_amsterdamtobogotaattime15k
FRAMES_STAMP := $(SPECTEST_VECTORS_DIR)/.frames_overlay
VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR)

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	$(DOWNLOAD) $(SPECTEST_URL) $(SPECTEST_ARTIFACT)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	rm -rf $(SPECTEST_VECTORS_DIR)
	mkdir -p $(SPECTEST_VECTORS_DIR)
	tar -xzf $(SPECTEST_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests_engine

help: ## 📚 Show help for each Makefile target
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

# Keyed on the release URL, exactly like the blockchain runner's frames overlay:
# a subtree left on disk says nothing in its mtime about which release produced
# it, and clearing it first drops fixtures a later release renamed or removed.
frames-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Overlay the frames-devnet engine fixtures, re-fetching when the pinned release changed
	@have=$$(cat $(FRAMES_STAMP) 2>/dev/null); \
	for subtree in $(FRAMES_SUBTREES); do \
		[ -d $(SPECTEST_VECTORS_DIR)/$$subtree ] || have=; \
	done; \
	if [ "$$have" = "$(FRAMES_URL)" ]; then \
		echo "Frames fixtures already overlaid from $(FRAMES_URL)"; \
	else \
		curl -fL -o $(FRAMES_ARTIFACT).part $(FRAMES_URL) && \
		mv $(FRAMES_ARTIFACT).part $(FRAMES_ARTIFACT) && \
		rm -rf $(addprefix $(SPECTEST_VECTORS_DIR)/,$(FRAMES_SUBTREES)) && \
		tar -xzf $(FRAMES_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
			$(addprefix fixtures/blockchain_tests_engine/,$(FRAMES_SUBTREES)) && \
		echo "$(FRAMES_URL)" > $(FRAMES_STAMP); \
	fi

download-test-vectors: $(VECTORS_TARGETS) frames-vectors ## 📥 Download engine-format test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(FRAMES_ARTIFACT) $(FRAMES_ARTIFACT).part

test: $(VECTORS_TARGETS) frames-vectors ## 🧪 Run engine-format fixture suite (InMemory, release)
	cargo test -p ef_tests-engine --release

test-rocksdb: $(VECTORS_TARGETS) frames-vectors ## 🧪 Run with RocksDB backend
	cargo test -p ef_tests-engine --release --features rocksdb

test-strict: $(VECTORS_TARGETS) frames-vectors ## 🧪 Run with strict validationError checking
	ETHREX_ENGINE_STRICT_EXCEPTIONS=1 cargo test -p ef_tests-engine --release

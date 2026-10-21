.PHONY: download-test-vectors clean-vectors test test-rocksdb test-strict amsterdam-vectors heze-vectors help

VECTORS_ROOT := vectors
FIXTURES_FILE := ../.fixtures_url

# Retrying downloader: release downloads fail transiently often enough to be the
# top cause of red CI. See the script header for what each guard catches.
DOWNLOAD := ../../../.github/scripts/download.sh

SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

AMSTERDAM_FIXTURES_FILE := ../.fixtures_url_amsterdam
AMSTERDAM_ARTIFACT := amsterdam-tests.tar.gz
AMSTERDAM_URL := $(shell cat $(AMSTERDAM_FIXTURES_FILE))

# Subtrees the Amsterdam bundle contributes. `for_bpo2toamsterdamattime15k` holds the
# fork-activation fixtures (a header field appearing mid-chain, the gas schedule
# switching between blocks); it exists only in this bundle, so omitting it leaves the
# whole fork boundary untested.
AMSTERDAM_SUBTREES := for_amsterdam for_bpo2toamsterdamattime15k
AMSTERDAM_STAMP := $(SPECTEST_VECTORS_DIR)/.amsterdam_overlay

# heze-devnet engine fixtures: EIP-7805 (FOCIL) and EIP-8198 (quick slots) at
# the "Bogota" network, Hegota in ethrex. The tarball nests them under
# fixtures/blockchain_tests_engine/for_bogota/ and, for the EIP-8198 fork
# transition, for_amsterdamtobogotaattime15k/. The datatest harness walks
# vectors/eest/ recursively, so they're discovered wherever they land in the tree.
#
# The pinned release, tests-heze-devnet@v3.0.0, is filled from execution-specs
# devnets/heze/3 (forks/bogota through execution-specs#3880), whose Amsterdam
# base is newer than the glamsterdam-devnet-8 one this branch implements. The
# one rule the bundle depends on that entered forks/amsterdam after devnet-8,
# the TX_MAX_TOTAL_GAS_LIMIT cap (execution-specs#3582), is therefore enforced
# from Hegota rather than Amsterdam, so the devnet-8 Amsterdam fixtures keep
# passing; only the Bogota subtrees are overlaid.
# Besides the eip7805_focil and eip8198_quick_slots scenarios, the bundle
# carries inclusion-list variants auto-generated from prior-fork
# blockchain/state tests: the last transaction of the last block moves into the
# inclusion list, expecting `inclusionListSatisfied: false` when it was
# includable and `true` when it was not (execution-specs#3337).
#
# Fixtures whose expected verdict contradicts the EELS spec at the same tag
# are enumerated, with the exact failure signature the harness enforces, in
# tests/upstream_broken_fixtures.txt (empty at v3.0.0).
HEZE_FIXTURES_FILE := ../.fixtures_url_heze
HEZE_ARTIFACT := heze-tests.tar.gz
HEZE_URL := $(shell cat $(HEZE_FIXTURES_FILE))
HEZE_SUBTREES := for_bogota for_amsterdamtobogotaattime15k
HEZE_STAMP := $(SPECTEST_VECTORS_DIR)/.heze_overlay

VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR)

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	$(DOWNLOAD) $(SPECTEST_URL) $(SPECTEST_ARTIFACT)

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
# current client. $(DOWNLOAD) fetches through a temporary file, so a failed download
# leaves the previous bundle and stamp untouched instead of truncating them.
amsterdam-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Overlay Amsterdam fixtures, re-fetching when the pinned release changed
	@have=$$(cat $(AMSTERDAM_STAMP) 2>/dev/null); \
	for subtree in $(AMSTERDAM_SUBTREES); do \
		[ -d $(SPECTEST_VECTORS_DIR)/$$subtree ] || have=; \
	done; \
	if [ "$$have" = "$(AMSTERDAM_URL)" ]; then \
		echo "Amsterdam fixtures already overlaid from $(AMSTERDAM_URL)"; \
	else \
		$(DOWNLOAD) $(AMSTERDAM_URL) $(AMSTERDAM_ARTIFACT) && \
		rm -rf $(addprefix $(SPECTEST_VECTORS_DIR)/,$(AMSTERDAM_SUBTREES)) && \
		tar -xzf $(AMSTERDAM_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
			$(addprefix fixtures/blockchain_tests_engine/,$(AMSTERDAM_SUBTREES)) && \
		echo "$(AMSTERDAM_URL)" > $(AMSTERDAM_STAMP); \
	fi

# Same URL-keyed overlay as the Amsterdam bundle above, for the same reason: the
# heze-devnet fixtures move with the devnet, so a subtree already on disk says
# nothing about which release produced it.
heze-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Overlay heze-devnet (EIP-7805, EIP-8198) engine fixtures, re-fetching when the pinned release changed
	@have=$$(cat $(HEZE_STAMP) 2>/dev/null); \
	for subtree in $(HEZE_SUBTREES); do \
		[ -d $(SPECTEST_VECTORS_DIR)/$$subtree ] || have=; \
	done; \
	if [ "$$have" = "$(HEZE_URL)" ]; then \
		echo "heze-devnet fixtures already overlaid from $(HEZE_URL)"; \
	else \
		$(DOWNLOAD) $(HEZE_URL) $(HEZE_ARTIFACT) && \
		rm -rf $(addprefix $(SPECTEST_VECTORS_DIR)/,$(HEZE_SUBTREES)) && \
		tar -xzf $(HEZE_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
			$(addprefix fixtures/blockchain_tests_engine/,$(HEZE_SUBTREES)) && \
		echo "$(HEZE_URL)" > $(HEZE_STAMP); \
	fi

help: ## 📚 Show help for each Makefile target
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors heze-vectors ## 📥 Download engine-format test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_ARTIFACT).part \
		$(HEZE_ARTIFACT) $(HEZE_ARTIFACT).part

test: $(VECTORS_TARGETS) amsterdam-vectors heze-vectors ## 🧪 Run engine-format fixture suite (InMemory, release)
	cargo test -p ef_tests-engine --release

test-rocksdb: $(VECTORS_TARGETS) amsterdam-vectors heze-vectors ## 🧪 Run with RocksDB backend
	cargo test -p ef_tests-engine --release --features rocksdb

test-strict: $(VECTORS_TARGETS) amsterdam-vectors heze-vectors ## 🧪 Run with strict validationError checking
	ETHREX_ENGINE_STRICT_EXCEPTIONS=1 cargo test -p ef_tests-engine --release

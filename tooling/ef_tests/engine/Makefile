.PHONY: download-test-vectors clean-vectors test test-rocksdb test-strict amsterdam-vectors focil-vectors help

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

# EIP-7805 (FOCIL) engine fixtures. The tarball nests them under
# fixtures/blockchain_tests_engine/for_bogota/; the datatest harness walks
# vectors/eest/ recursively, so they are discovered wherever they land.
#
# The pinned release, tests-focil-devnet@v0.2.0, fills every fixture on the
# "Bogota" network -- EIP-7805 on top of Amsterdam. The Amsterdam it is filled
# on is the one $(AMSTERDAM_FIXTURES_FILE) already pins: diffed against
# tests-glamsterdam-devnet@v8.1.1 the Amsterdam fork spec at the focil tag is two
# files and +96/-2 lines, all of them FOCIL, while against v8.1.0 it is twenty
# files and +428/-261. Keep the two pins on the same base when bumping either:
# a focil bundle filled on a different Amsterdam fails on base-EIP expectations
# that have nothing to do with FOCIL.
#
# Besides the eip7805_focil scenarios, the bundle carries inclusion-list variants
# auto-generated from prior-fork blockchain and state tests: the last transaction
# of the last block moves into the inclusion list, expecting
# `inclusionListSatisfied: false` when it was includable and `true` when it was
# not (execution-specs#3337).
#
# 127 of those generated variants ship expected-VALID blocks that the EELS spec
# at the same tag rejects (stale `blobGasUsed` left by the variant generator; one
# block whose access list exceeds the EIP-7928 item cap at its pinned 21k gas
# limit). They are enumerated -- with the analysis and the exact failure
# signature the harness enforces -- in tests/upstream_broken_fixtures.txt.
FOCIL_FIXTURES_FILE := .fixtures_url_focil
FOCIL_ARTIFACT := focil-tests.tar.gz
FOCIL_URL := $(shell cat $(FOCIL_FIXTURES_FILE))
FOCIL_SUBTREES := for_bogota
FOCIL_STAMP := $(SPECTEST_VECTORS_DIR)/.focil_overlay

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
# FOCIL fixtures move with the devnet, so a subtree already on disk says nothing
# about which release produced it.
focil-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Overlay EIP-7805 FOCIL engine fixtures, re-fetching when the pinned release changed
	@have=$$(cat $(FOCIL_STAMP) 2>/dev/null); \
	for subtree in $(FOCIL_SUBTREES); do \
		[ -d $(SPECTEST_VECTORS_DIR)/$$subtree ] || have=; \
	done; \
	if [ "$$have" = "$(FOCIL_URL)" ]; then \
		echo "FOCIL fixtures already overlaid from $(FOCIL_URL)"; \
	else \
		$(DOWNLOAD) $(FOCIL_URL) $(FOCIL_ARTIFACT) && \
		rm -rf $(addprefix $(SPECTEST_VECTORS_DIR)/,$(FOCIL_SUBTREES)) && \
		tar -xzf $(FOCIL_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) \
			$(addprefix fixtures/blockchain_tests_engine/,$(FOCIL_SUBTREES)) && \
		echo "$(FOCIL_URL)" > $(FOCIL_STAMP); \
	fi

help: ## 📚 Show help for each Makefile target
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 📥 Download engine-format test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_ARTIFACT).part \
		$(FOCIL_ARTIFACT) $(FOCIL_ARTIFACT).part

test: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 🧪 Run engine-format fixture suite (InMemory, release)
	cargo test -p ef_tests-engine --release

test-rocksdb: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 🧪 Run with RocksDB backend
	cargo test -p ef_tests-engine --release --features rocksdb

test-strict: $(VECTORS_TARGETS) amsterdam-vectors focil-vectors ## 🧪 Run with strict validationError checking
	ETHREX_ENGINE_STRICT_EXCEPTIONS=1 cargo test -p ef_tests-engine --release

.PHONY: download-test-vectors clean-vectors test test-levm test-sp1 test-stateless amsterdam-vectors zkevm-vectors test-stateless-zkevm frames-vectors

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
# EIP-8141 frame transactions have no released fixture bundle: the EELS
# implementation is still an open PR, so the fixtures are filled locally from a
# pinned revision of it. `--fork Bogota` is the pseudo-fork (Amsterdam + EIP-8141)
# those tests are labelled with; it is outside execution-specs' default fill
# range, which is why the fork bounds are given explicitly. Drop this target and
# fold the subtree into the Amsterdam bundle once a release carries it.
FRAMES_SPEC_REV_FILE := ../.frames_spec_rev
FRAMES_SPEC_REV := $(shell cat $(FRAMES_SPEC_REV_FILE))
FRAMES_SPEC_REPO ?= https://github.com/ethereum/execution-specs.git
FRAMES_SPEC_DIR ?= $(CURDIR)/.execution-specs
FRAMES_TESTS := tests/amsterdam/eip8141_frame_transactions
FRAMES_SUBTREE := for_bogota
FRAMES_STAMP := $(SPECTEST_VECTORS_DIR)/.frames_rev

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
			$(addprefix fixtures/blockchain_tests/,$(AMSTERDAM_SUBTREES)) && \
		echo "$(AMSTERDAM_URL)" > $(AMSTERDAM_STAMP); \
	fi

$(ZKEVM_ARTIFACT): $(ZKEVM_FIXTURES_FILE)
	curl -L -o $(ZKEVM_ARTIFACT) $(ZKEVM_URL)

$(ZKEVM_VECTORS_DIR): $(ZKEVM_ARTIFACT)
	rm -rf $(ZKEVM_VECTORS_DIR)
	mkdir -p $(ZKEVM_VECTORS_DIR)
	tar -xzf $(ZKEVM_ARTIFACT) --strip-components=2 -C $(ZKEVM_VECTORS_DIR) fixtures/blockchain_tests/for_amsterdam

zkevm-vectors: $(ZKEVM_VECTORS_DIR)

# Keyed on the filled revision, like the Amsterdam overlay: a `for_bogota/` tree
# left by another branch says nothing in its mtime about which spec revision
# filled it. Requires `uv` (execution-specs' toolchain manager).
frames-vectors: | $(SPECTEST_VECTORS_DIR) ## 📥 Fill EIP-8141 frame fixtures from the pinned execution-specs revision
	@have=$$(cat $(FRAMES_STAMP) 2>/dev/null); \
	[ -d $(SPECTEST_VECTORS_DIR)/$(FRAMES_SUBTREE) ] || have=; \
	if [ "$$have" = "$(FRAMES_SPEC_REV)" ]; then \
		echo "Frame fixtures already filled from $(FRAMES_SPEC_REV)"; \
	else \
		command -v uv >/dev/null || { echo "frames-vectors needs uv: https://docs.astral.sh/uv/"; exit 1; }; \
		[ -d $(FRAMES_SPEC_DIR) ] || git clone --filter=blob:none $(FRAMES_SPEC_REPO) $(FRAMES_SPEC_DIR); \
		git -C $(FRAMES_SPEC_DIR) fetch --quiet origin $(FRAMES_SPEC_REV) && \
		git -C $(FRAMES_SPEC_DIR) checkout --quiet $(FRAMES_SPEC_REV) && \
		(cd $(FRAMES_SPEC_DIR) && uv sync --quiet && \
			uv run fill $(FRAMES_TESTS) --from Bogota --until Bogota \
				--output $(FRAMES_SPEC_DIR)/.frames-fixtures --clean) && \
		rm -rf $(SPECTEST_VECTORS_DIR)/$(FRAMES_SUBTREE) && \
		cp -R $(FRAMES_SPEC_DIR)/.frames-fixtures/blockchain_tests/$(FRAMES_SUBTREE) \
			$(SPECTEST_VECTORS_DIR)/$(FRAMES_SUBTREE) && \
		echo "$(FRAMES_SPEC_REV)" > $(FRAMES_STAMP); \
	fi

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors zkevm-vectors ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT) $(ZKEVM_VECTORS_ROOT) $(FRAMES_SPEC_DIR)
	rm -f $(SPECTEST_ARTIFACT) $(LEGACYTEST_ARTIFACT) $(AMSTERDAM_ARTIFACT) $(AMSTERDAM_ARTIFACT).part $(ZKEVM_ARTIFACT)

test-levm: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors ## 🧪 Run blockchain tests with LEVM
	cargo test --profile release-fast

test-sp1: $(VECTORS_TARGETS) amsterdam-vectors frames-vectors
	cargo test --profile release-fast --features sp1

test-stateless: zkevm-vectors
	cargo test --profile release-fast --features stateless

test-stateless-zkevm: zkevm-vectors
	cargo test --profile release-fast --features stateless -- eip8025_optional_proofs

test: ## 🧪 Run blockchain tests with LEVM both with state and stateless
	$(MAKE) test-levm
	$(MAKE) test-stateless

.PHONY: download-test-vectors clean-vectors test test-rocksdb test-strict amsterdam-vectors focil-vectors help

VECTORS_ROOT := vectors
FIXTURES_FILE := ../.fixtures_url
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
# fixtures/blockchain_tests_engine/for_bogota/. The datatest harness walks
# vectors/eest/ recursively, so they're discovered wherever they land in the tree.
#
# KNOWN RED as of 2026-08-12, and not because of the FOCIL code. Two separate
# upstream gaps, both established by experiment rather than inference:
#
# 1. The pinned release, tests-focil@v0.1.0, was filled on 2026-06-22 from
#    execution-specs `devnets/focil/0`, whose Amsterdam base is two months
#    behind the one this branch implements. Its genesis predeploys no EIP-8282
#    builder deposit/exit contracts -- and EIP-8282 says "if there is no code at
#    either address once the EIP is active, every block from activation onward
#    MUST be invalid". So ethrex rejecting all 24 is SPEC-CORRECT; the fixtures
#    simply cannot be run against a devnet-8 client. (Bypassing that rule
#    locally leaves 17 failing on a pre-EIP-8038 coinbase balance in the BAL.)
#
# 2. The successor branch, `eips/bogota/eip-7805` @ e5dc8456f, does not fill as
#    published: t8n hands `check_inclusion_list_transactions` decoded
#    transactions where it expects `LegacyTransaction | Bytes`, and its blocks
#    size the gas limit to the transactions alone, leaving no budget for the
#    EIP-7928 access list. With both repaired locally (patch kept alongside this
#    branch's notes) all 24 fill, and **ethrex passes 23 of them**.
#
#    The 24th, `unsatisfied_with_contract_creating_pending_il_tx`, fails on a
#    gas-schedule difference, not on FOCIL: that branch's Amsterdam is NEWER
#    than glamsterdam-devnet-8 (COLD_STORAGE_ACCESS 3000 vs 2100, ACCOUNT_WRITE
#    8000 vs 9000, CREATE_ACCESS built from COLD_STORAGE_ACCESS rather than
#    COLD_ACCOUNT_ACCESS, TX_VALUE_COST split into 4244+1756 rather than 6000),
#    so its contract-creation intrinsic is 23000 where devnet-8 -- and ethrex --
#    charge 24000. The IL transaction is therefore appendable there and not
#    here.
#
# Upstream has said EELS/EEST will be rebased onto devnet-8; re-pin FOCIL_URL to
# that release when it lands and this should go green without client changes.
# Coverage meanwhile comes from the in-repo suite
# (test/tests/blockchain/focil_tests.rs and the inclusion_list_* tests).
FOCIL_FIXTURES_FILE := .fixtures_url_focil
FOCIL_ARTIFACT := focil-tests.tar.gz
FOCIL_URL := $(shell cat $(FOCIL_FIXTURES_FILE))
FOCIL_SUBTREES := for_bogota
FOCIL_STAMP := $(SPECTEST_VECTORS_DIR)/.focil_overlay

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
		curl -fL -o $(FOCIL_ARTIFACT).part $(FOCIL_URL) && \
		mv $(FOCIL_ARTIFACT).part $(FOCIL_ARTIFACT) && \
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

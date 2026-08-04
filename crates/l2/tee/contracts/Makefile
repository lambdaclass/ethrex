DETERMINISTIC_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C
DEPLOYMENT_PATH := deploydeps/automata-dcap-attestation/evm/deployment

ETHREX_TDX_DEV_MODE ?= false
ON_CHAIN_PROPOSER ?= 0x0

# Pinned revisions for every cloned dependency. These clones are gitignored, so
# what is recorded here is the only in-tree record of what a deployed verifier was
# built from.
#
# Bumping any of these changes on-chain verification behaviour: re-check the byte
# offsets in src/TDXVerifier.sol against the attestation output layout first, since
# _validateReport indexes into it with hardcoded offsets.
#
# Comments stay off the value lines: Make keeps whitespace before an inline `#`.
# The two forks pin their own openzeppelin/solady, so these fix the whole
# transitive contract tree.
# lambdaclass/automata-on-chain-pccs, merge commit of its PR #2, on main.
PCCS_REV = 903fd1bcfd4609f7d7a2c0cb809a3240a927db2b
# lambdaclass/automata-dcap-attestation.
# NOTE: still a commit on that fork's `chore/pin-dependency-revisions` branch (its
# PR #2), not main. Repoint at the merge commit once it lands, so a default clone
# can still reach it if the branch is deleted.
DCAP_ATTESTATION_REV = 44b5a6def41c9f482402bab65ed84134ef369f1f
# OpenZeppelin/openzeppelin-contracts v5.7.0
OPENZEPPELIN_REV = cab19933c33c2ad1d4c7a84864a3601dddfd16f3
# automata-network/automata-dcap-qpl, main tip as of 2026-08-03
DCAP_QPL_REV = 3877d9aab99c090ad63bc71c18878e7659e26695
# automata-network/pccs-reader-rs, main tip as of 2026-08-03
PCCS_READER_REV = 926025f12cb63b6c1c50d5aad735223e71ed853b

# Clone if absent, then re-assert and verify the pin on every run, so a stale or
# half-checked-out tree is corrected rather than silently reused. After editing a
# revision above, `make clean` to rebuild.
define pin
	@test -d $(1)/.git || git clone -q $(2) $(1)
	@git -C $(1) cat-file -e "$(3)^{commit}" 2>/dev/null || git -C $(1) fetch -q --tags origin
	@git -c advice.detachedHead=false -C $(1) checkout -q --detach $(3)
	@test "$$(git -C $(1) rev-parse HEAD)" = "$(3)" || { echo "$(1): not at $(3)"; exit 1; }
endef

deploy-p256:
	rex send $(DETERMINISTIC_DEPLOYER) -k $(PRIVATE_KEY) --calldata $(shell cat assets/p256.hex)

.PHONY: deploydeps
deploydeps:
	mkdir -p deploydeps
	$(call pin,deploydeps/automata-on-chain-pccs,https://github.com/lambdaclass/automata-on-chain-pccs.git,$(PCCS_REV))
	$(call pin,deploydeps/automata-dcap-attestation,https://github.com/lambdaclass/automata-dcap-attestation.git,$(DCAP_ATTESTATION_REV))

deploy-pccs: deploydeps deploy-p256
	cd deploydeps/automata-on-chain-pccs; make deploy

deploy-dcap: deploydeps deploy-pccs
	mkdir -p $(DEPLOYMENT_PATH)
	cp deploydeps/automata-on-chain-pccs/deployment/* $(DEPLOYMENT_PATH)
	cd deploydeps/automata-dcap-attestation/evm; make deploy

ROOT_CRL_URI = https://certificates.trustedservices.intel.com/IntelSGXRootCA.der
# SGX and TDX roots are the same
deploydeps/root_crl.hex: | deploydeps
	curl $(ROOT_CRL_URI) | xxd -ps -c0 > deploydeps/root_crl.hex

ROOT_CA_URI = https://certificates.trustedservices.intel.com/Intel_SGX_Provisioning_Certification_RootCA.cer
# SGX and TDX roots are the same
deploydeps/root_ca.hex: | deploydeps
	curl $(ROOT_CA_URI) | xxd -ps -c0 > deploydeps/root_ca.hex

setup-pccs-ca: deploy-pccs deploydeps/root_ca.hex deploydeps/root_crl.hex
	$(eval PCSDAO_ADDRESS := $(shell cat ${DEPLOYMENT_PATH}/AutomataPcsDao))
	rex send $(PCSDAO_ADDRESS) "upsertPcsCertificates(uint8,bytes)" 0 $(shell cat deploydeps/root_ca.hex) -k $(PRIVATE_KEY)
	rex send $(PCSDAO_ADDRESS) "upsertRootCACrl(bytes)" $(shell cat deploydeps/root_crl.hex) -k $(PRIVATE_KEY)
	rex send $(PCSDAO_ADDRESS) "upsertPcsCertificates(uint8,bytes)" 2 $(shell cat assets/platform_ca.hex) -k $(PRIVATE_KEY)

.PHONY: pin-openzeppelin
pin-openzeppelin:
	mkdir -p lib
	$(call pin,lib/openzeppelin-contracts,https://github.com/OpenZeppelin/openzeppelin-contracts,$(OPENZEPPELIN_REV))

solc_out/TDXVerifier.bin: src/TDXVerifier.sol | pin-openzeppelin
	mkdir -p solc_out
	solc src/TDXVerifier.sol --bin --allow-paths lib/,../../.. --base-path . --include-path ../../ @openzeppelin/=lib/openzeppelin-contracts/ -o solc_out/ --overwrite

deploy: solc_out/TDXVerifier.bin
	$(eval CONTRACT_BIN := $(shell cat solc_out/TDXVerifier.bin))
	$(eval DCAP_ADDRESS := $(shell cat ${DEPLOYMENT_PATH}/AutomataDcapAttestationFee))
	rex deploy --print-address $(CONTRACT_BIN) 0 $(PRIVATE_KEY) -- \
		"constructor(address,address,bool)" $(DCAP_ADDRESS) $(ON_CHAIN_PROPOSER) $(ETHREX_TDX_DEV_MODE) \
		> ${DEPLOYMENT_PATH}/TDXVerifier

deploy-deps: deploy-dcap setup-pccs-ca

# Deliberately NOT re-asserted on every run like the targets above: the recipe
# edits its own checkout (removes Cargo.toml, appends a [workspace] section), so a
# repeated checkout would fight those edits. This builds a collateral-fetching CLI
# and is not an input to any deployed bytecode. `make clean` to re-pin it.
automata-dcap-qpl:
	git clone https://github.com/automata-network/automata-dcap-qpl
	git -C automata-dcap-qpl checkout --detach $(DCAP_QPL_REV)
	cd automata-dcap-qpl/automata-dcap-qpl-tool; git clone https://github.com/automata-network/pccs-reader-rs
	git -C automata-dcap-qpl/automata-dcap-qpl-tool/pccs-reader-rs checkout --detach $(PCCS_READER_REV)
	rm automata-dcap-qpl/Cargo.toml
	echo "" >> automata-dcap-qpl/automata-dcap-qpl-tool/Cargo.toml
	echo "[workspace]" >> automata-dcap-qpl/automata-dcap-qpl-tool/Cargo.toml
	cd automata-dcap-qpl/automata-dcap-qpl-tool; cargo build --release

deploy-all:
	make deploy-deps
	make deploy
	make automata-dcap-qpl

clean:
	rm -rf deploydeps cache out deployment/* automata-dcap-qpl

.PHONY: deploy-all deploy deploy-pccs deploy-dcap clean

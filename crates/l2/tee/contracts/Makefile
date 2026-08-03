DETERMINISTIC_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C
DEPLOYMENT_PATH := deploydeps/automata-dcap-attestation/evm/deployment

ETHREX_TDX_DEV_MODE ?= false
ON_CHAIN_PROPOSER ?= 0x0

# Pinned revisions for every cloned dependency.
#
# These clones are gitignored, so the revision recorded here is the only in-tree
# record of what the deployed verifier was built from. Leaving them unpinned means
# `make deploy-all` can produce different bytecode on different days, and makes it
# impossible to tell after the fact which version of the DCAP attestation logic a
# given deployment is running.
#
# Bumping any of these changes on-chain verification behaviour: re-check the byte
# offsets in src/TDXVerifier.sol against the attestation output layout before doing
# so, since _validateReport indexes into it with hardcoded offsets.
#
# Keep each comment on its own line: Make preserves whitespace preceding an
# inline `#`, which would be appended to the revision string.
# The two forks below pin their own openzeppelin/solady clones as of these
# revisions, so pinning them here fixes the whole transitive contract tree.
# Both currently sit on the fork branch `chore/pin-dependency-revisions` rather
# than main: re-point them at the merge commits once those PRs land, so the
# revisions stay reachable if the branches are deleted.
# lambdaclass/automata-on-chain-pccs
PCCS_REV = 41a4422ffad3e8b0b5cf1c210ea9aa2c9f51adb4
# lambdaclass/automata-dcap-attestation
DCAP_ATTESTATION_REV = e09b587c214f016c9ee5c9b7f71e0bb581c84134
# OpenZeppelin/openzeppelin-contracts v5.7.0
OPENZEPPELIN_REV = cab19933c33c2ad1d4c7a84864a3601dddfd16f3
# automata-network/automata-dcap-qpl, main tip as of 2026-08-03
DCAP_QPL_REV = 3877d9aab99c090ad63bc71c18878e7659e26695
# automata-network/pccs-reader-rs, main tip as of 2026-08-03
PCCS_READER_REV = 926025f12cb63b6c1c50d5aad735223e71ed853b

deploy-p256:
	rex send $(DETERMINISTIC_DEPLOYER) -k $(PRIVATE_KEY) --calldata $(shell cat assets/p256.hex)

deploydeps:
	mkdir -p deploydeps
	cd deploydeps; git clone https://github.com/lambdaclass/automata-on-chain-pccs.git
	git -C deploydeps/automata-on-chain-pccs checkout --detach $(PCCS_REV)
	cd deploydeps; git clone https://github.com/lambdaclass/automata-dcap-attestation.git
	git -C deploydeps/automata-dcap-attestation checkout --detach $(DCAP_ATTESTATION_REV)

deploy-pccs: deploydeps deploy-p256
	cd deploydeps/automata-on-chain-pccs; make deploy

deploy-dcap: deploydeps deploy-pccs
	mkdir -p $(DEPLOYMENT_PATH)
	cp deploydeps/automata-on-chain-pccs/deployment/* $(DEPLOYMENT_PATH)
	cd deploydeps/automata-dcap-attestation/evm; make deploy

ROOT_CRL_URI = https://certificates.trustedservices.intel.com/IntelSGXRootCA.der
deploydeps/root_crl.hex:
	# SGX and TDX roots are the same
	curl $(ROOT_CRL_URI) | xxd -ps -c0 > deploydeps/root_crl.hex

ROOT_CA_URI = https://certificates.trustedservices.intel.com/Intel_SGX_Provisioning_Certification_RootCA.cer
deploydeps/root_ca.hex:
	# SGX and TDX roots are the same
	curl $(ROOT_CA_URI) | xxd -ps -c0 > deploydeps/root_ca.hex

setup-pccs-ca: deploy-pccs deploydeps/root_ca.hex deploydeps/root_crl.hex
	$(eval PCSDAO_ADDRESS := $(shell cat ${DEPLOYMENT_PATH}/AutomataPcsDao))
	rex send $(PCSDAO_ADDRESS) "upsertPcsCertificates(uint8,bytes)" 0 $(shell cat deploydeps/root_ca.hex) -k $(PRIVATE_KEY)
	rex send $(PCSDAO_ADDRESS) "upsertRootCACrl(bytes)" $(shell cat deploydeps/root_crl.hex) -k $(PRIVATE_KEY)
	rex send $(PCSDAO_ADDRESS) "upsertPcsCertificates(uint8,bytes)" 2 $(shell cat assets/platform_ca.hex) -k $(PRIVATE_KEY)

lib/openzeppelin-contracts:
	mkdir -p lib
	cd lib; git clone https://github.com/OpenZeppelin/openzeppelin-contracts
	git -C lib/openzeppelin-contracts checkout --detach $(OPENZEPPELIN_REV)

solc_out/TDXVerifier.bin: src/TDXVerifier.sol lib/openzeppelin-contracts
	mkdir -p solc_out
	solc src/TDXVerifier.sol --bin --allow-paths lib/,../../.. --base-path . --include-path ../../ @openzeppelin/=lib/openzeppelin-contracts/ -o solc_out/ --overwrite

deploy: solc_out/TDXVerifier.bin
	$(eval CONTRACT_BIN := $(shell cat solc_out/TDXVerifier.bin))
	$(eval DCAP_ADDRESS := $(shell cat ${DEPLOYMENT_PATH}/AutomataDcapAttestationFee))
	rex deploy --print-address $(CONTRACT_BIN) 0 $(PRIVATE_KEY) -- \
		"constructor(address,address,bool)" $(DCAP_ADDRESS) $(ON_CHAIN_PROPOSER) $(ETHREX_TDX_DEV_MODE) \
		> ${DEPLOYMENT_PATH}/TDXVerifier

deploy-deps: deploy-dcap setup-pccs-ca

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

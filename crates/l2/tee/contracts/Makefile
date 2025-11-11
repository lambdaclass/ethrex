DETERMINISTIC_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C
DEPLOYMENT_PATH := deploydeps/automata-dcap-attestation/evm/deployment

ETHREX_TDX_DEV_MODE ?= false
ON_CHAIN_PROPOSER ?= 0x0

deploy-p256:
	rex send $(DETERMINISTIC_DEPLOYER) -k $(PRIVATE_KEY) --calldata $(shell cat assets/p256.hex)

deploydeps:
	mkdir -p deploydeps
	cd deploydeps; git clone https://github.com/lambdaclass/automata-on-chain-pccs.git
	cd deploydeps; git clone https://github.com/lambdaclass/automata-dcap-attestation.git

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

solc_out/TDXVerifier.bin: src/TDXVerifier.sol lib/openzeppelin-contracts
	mkdir -p solc_out
	solc src/TDXVerifier.sol --bin --allow-paths lib/ -o solc_out/ --overwrite

deploy: solc_out/TDXVerifier.bin
	$(eval CONTRACT_BIN := $(shell cat solc_out/TDXVerifier.bin))
	$(eval DCAP_ADDRESS := $(shell cat ${DEPLOYMENT_PATH}/AutomataDcapAttestationFee))
	rex deploy --print-address $(CONTRACT_BIN) 0 $(PRIVATE_KEY) -- \
		"constructor(address,address,bool)" $(DCAP_ADDRESS) $(ON_CHAIN_PROPOSER) $(ETHREX_TDX_DEV_MODE) \
		> ${DEPLOYMENT_PATH}/TDXVerifier

deploy-deps: deploy-dcap setup-pccs-ca

automata-dcap-qpl:
	git clone https://github.com/automata-network/automata-dcap-qpl
	cd automata-dcap-qpl/automata-dcap-qpl-tool; git clone https://github.com/automata-network/pccs-reader-rs
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

DETERMINISTIC_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C
CHAIN_ID := $(shell cast chain-id --rpc-url $(RPC_URL))
PCCS_FILE := deploydeps/automata-dcap-attestation/evm/deployment/$(CHAIN_ID)/onchain_pccs.json
DCAP_FILE := deploydeps/automata-dcap-attestation/evm/deployment/$(CHAIN_ID)/dcap.json

deploy-p256:
	cast send $(DETERMINISTIC_DEPLOYER) $(shell cat assets/p256.hex) \
		--rpc-url $(RPC_URL) \
		--private-key $(PRIVATE_KEY)

deploydeps:
	mkdir -p deploydeps
	cd deploydeps; git clone https://github.com/automata-network/automata-on-chain-pccs.git
	cd deploydeps; git clone https://github.com/automata-network/automata-dcap-attestation.git

deploy-pccs: deploydeps deploy-p256
	cd deploydeps/automata-on-chain-pccs; git submodule update --init
	cd deploydeps/automata-on-chain-pccs; forge install
	cd deploydeps/automata-on-chain-pccs; make deploy-all

deploy-dcap: deploydeps deploy-pccs
	mkdir -p deploydeps/automata-dcap-attestation/evm/deployment/$(CHAIN_ID)
	cp deploydeps/automata-on-chain-pccs/deployment/$(CHAIN_ID).json deploydeps/automata-dcap-attestation/evm/deployment/$(CHAIN_ID)/onchain_pccs.json
	cd deploydeps/automata-dcap-attestation/evm; git submodule update --init
	cd deploydeps/automata-dcap-attestation/evm; forge install
	cd deploydeps/automata-dcap-attestation/evm; make deploy-all

ROOT_CRL_URI = https://certificates.trustedservices.intel.com/IntelSGXRootCA.der
deploydeps/root_crl.hex:
	# SGX and TDX roots are the same
	curl $(ROOT_CRL_URI) | xxd -ps -c0 > deploydeps/root_crl.hex

ROOT_CA_URI = https://certificates.trustedservices.intel.com/Intel_SGX_Provisioning_Certification_RootCA.cer
deploydeps/root_ca.hex:
	# SGX and TDX roots are the same
	curl $(ROOT_CA_URI) | xxd -ps -c0 > deploydeps/root_ca.hex

setup-pccs-ca: deploy-pccs deploydeps/root_ca.hex deploydeps/root_crl.hex
	$(eval PCSDAO_ADDRESS := $(shell cat ${PCCS_FILE} | jq --raw-output .AutomataPcsDao))
	cast send $(PCSDAO_ADDRESS) "upsertPcsCertificates(uint8,bytes)" 0 $(shell cat deploydeps/root_ca.hex) \
		--rpc-url $(RPC_URL) \
		--private-key $(PRIVATE_KEY)
	cast send $(PCSDAO_ADDRESS) "upsertRootCACrl(bytes)" $(shell cat deploydeps/root_crl.hex) \
		--rpc-url $(RPC_URL) \
		--private-key $(PRIVATE_KEY)
	cast send $(PCSDAO_ADDRESS) "upsertPcsCertificates(uint8,bytes)" 2 $(shell cat assets/platform_ca.hex) \
		--rpc-url $(RPC_URL) \
		--private-key $(PRIVATE_KEY)

set-verifier-v4: deploy-dcap
	$(eval DCAP_ADDRESS := $(shell cat ${DCAP_FILE} | jq --raw-output .AutomataDcapAttestationFee))
	$(eval VERIFIER_V4_ADDR := $(shell cat ${DCAP_FILE} | jq --raw-output .V4QuoteVerifier))
	cast send $(DCAP_ADDRESS) "setQuoteVerifier(address)" $(VERIFIER_V4_ADDR) \
		--rpc-url $(RPC_URL) \
		--private-key $(PRIVATE_KEY)

lib/openzeppelin-contracts:
	mkdir -p lib
	cd lib; git clone https://github.com/OpenZeppelin/openzeppelin-contracts

solc_out/Counter.bin: src/Counter.sol lib/openzeppelin-contracts
	mkdir -p solc_out
	solc src/Counter.sol --bin --allow-paths lib/ -o solc_out/

deploy: solc_out/Counter.bin
	$(eval CONTRACT_BIN := $(shell cat solc_out/Counter.bin))
	$(eval DCAP_ADDRESS := $(shell cat ${DCAP_FILE} | jq --raw-output .AutomataDcapAttestationFee))
	cast send  \
		--rpc-url $(RPC_URL) \
		--private-key $(PRIVATE_KEY) \
		--create $(CONTRACT_BIN) "constructor(address)" $(DCAP_ADDRESS)

deploy-deps: deploy-dcap set-verifier-v4 setup-pccs-ca

clean:
	rm -rf deploydeps cache out deployment/*

.PHONY: deploy-all deploy deploy-pccs deploy-dcap set-verifier-v4 clean

DETERMINISTIC_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C
DEPLOYMENT_PATH := deploydeps/automata-dcap-attestation/evm/deployment

deploy-p256:
	rex send $(DETERMINISTIC_DEPLOYER) 0 $(PRIVATE_KEY) --calldata $(shell cat assets/p256.hex)

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
	rex send $(PCSDAO_ADDRESS) 0 $(PRIVATE_KEY) -- "upsertPcsCertificates(uint8,bytes)" 0 $(shell cat deploydeps/root_ca.hex)
	rex send $(PCSDAO_ADDRESS) 0 $(PRIVATE_KEY) -- "upsertRootCACrl(bytes)" $(shell cat deploydeps/root_crl.hex)
	rex send $(PCSDAO_ADDRESS) 0 $(PRIVATE_KEY) -- "upsertPcsCertificates(uint8,bytes)" 2 $(shell cat assets/platform_ca.hex)

lib/openzeppelin-contracts:
	mkdir -p lib
	cd lib; git clone https://github.com/OpenZeppelin/openzeppelin-contracts

solc_out/Counter.bin: src/Counter.sol lib/openzeppelin-contracts
	mkdir -p solc_out
	solc src/Counter.sol --bin --allow-paths lib/ -o solc_out/ --overwrite

deploy: solc_out/Counter.bin
	$(eval CONTRACT_BIN := $(shell cat solc_out/Counter.bin))
	$(eval DCAP_ADDRESS := $(shell cat ${DEPLOYMENT_PATH}/AutomataDcapAttestationFee))
	rex deploy --print-address $(CONTRACT_BIN) 0 $(PRIVATE_KEY) -- \
		"constructor(address)" $(DCAP_ADDRESS) > ${DEPLOYMENT_PATH}/Counter

mkenv:
	echo CONTRACT_ADDRESS=$(shell cat ${DEPLOYMENT_PATH}/Counter) > .env.out
	echo ENCLAVE_ID_DAO=$(shell cat ${DEPLOYMENT_PATH}/AutomataEnclaveIdentityDao)  >> .env.out
	echo FMSPC_TCB_DAO=$(shell cat ${DEPLOYMENT_PATH}/AutomataFmspcTcbDao)  >> .env.out
	echo PCK_DAO=$(shell cat ${DEPLOYMENT_PATH}/AutomataPckDao)  >> .env.out
	echo PCS_DAO=$(shell cat ${DEPLOYMENT_PATH}/AutomataPcsDao)  >> .env.out

deploy-deps: deploy-dcap setup-pccs-ca

clean:
	rm -rf deploydeps cache out deployment/*

.PHONY: deploy-all deploy deploy-pccs deploy-dcap clean

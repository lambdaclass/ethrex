.PHONY: prove-sp1-gpu-ci prove-risc0-gpu-ci

NETWORK ?= mainnet

# Block parameters
ifdef BLOCK_NUMBER
REPLAY_BLOCK_ARGS = ${BLOCK_NUMBER}
endif
REPLAY_BLOCK_ARGS += --rpc-url ${RPC_URL} --network ${NETWORK} --bench

prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- block --prove ${REPLAY_BLOCK_ARGS}
prove-risc0-gpu-ci:
	cargo r -r --no-default-features --features "risc0,gpu,ci" -- block --prove ${REPLAY_BLOCK_ARGS}

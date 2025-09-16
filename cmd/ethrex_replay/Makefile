.PHONY: prove-sp1-gpu-ci prove-risc0-gpu-ci

# Block parameters
ifdef BLOCK_NUMBER
REPLAY_BLOCK_ARGS = --block ${BLOCK_NUMBER}
endif
REPLAY_BLOCK_ARGS += --rpc-url ${RPC_URL} --bench

## Execution block
execute:
	cargo r -r --features ci --no-default-features -- block --execute ${REPLAY_BLOCK_ARGS}

prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- block --prove ${REPLAY_BLOCK_ARGS}
prove-risc0-gpu-ci:
	cargo r -r --no-default-features --features "risc0,gpu,ci" -- block --prove ${REPLAY_BLOCK_ARGS}
prove-openvm-gpu-ci:
	cargo r -r --no-default-features --features "openvm,gpu,ci" -- block --prove ${REPLAY_BLOCK_ARGS}

execute-sp1-ci:
	cargo r -r --features "sp1,ci" -- block --execute ${REPLAY_BLOCK_ARGS}
execute-risc0-ci:
	cargo r -r --no-default-features --features "risc0,ci" -- block --execute ${REPLAY_BLOCK_ARGS}
execute-openvm-ci:
	cargo r -r --no-default-features --features "openvm,ci" -- block --execute ${REPLAY_BLOCK_ARGS}

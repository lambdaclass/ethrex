.PHONY: execute prove-sp1-gpu-ci prove-risc0-gpu-ci execute-sp1-ci execute-risc0-ci

# Block parameters
ifdef BLOCK_NUMBER
REPLAY_BLOCK_ARGS = ${BLOCK_NUMBER}
endif
REPLAY_BLOCK_ARGS += --rpc-url ${RPC_URL}

## Execution block
execute:
	cargo r -r --no-default-features -- block ${REPLAY_BLOCK_ARGS}
  
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- block --zkvm sp1 --action prove --resource gpu ${REPLAY_BLOCK_ARGS} --bench
prove-risc0-gpu-ci:
	cargo r -r --no-default-features --features "risc0,gpu" -- block --zkvm risc0 --action prove --resource gpu ${REPLAY_BLOCK_ARGS} --bench

execute-sp1-ci:
	cargo r -r --features "sp1" -- block --zkvm sp1 ${REPLAY_BLOCK_ARGS} --bench
execute-risc0-ci:
	cargo r -r --no-default-features --features "risc0" -- block --zkvm risc0 ${REPLAY_BLOCK_ARGS} --bench

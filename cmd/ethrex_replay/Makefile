.PHONY: execute prove-sp1-gpu-ci prove-risc0-gpu-ci execute-sp1-ci execute-risc0-ci

# Block parameters
ifdef BLOCK_NUMBER
REPLAY_BLOCK_ARGS = ${BLOCK_NUMBER}
endif
REPLAY_BLOCK_ARGS += --rpc-url ${RPC_URL}

## Execution block
execute:
	cargo r -r --features ci --no-default-features -- block ${REPLAY_BLOCK_ARGS}
  
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- block --zkvm sp1 --action prove --resource gpu ${REPLAY_BLOCK_ARGS}
prove-risc0-gpu-ci:
	cargo r -r --no-default-features --features "risc0,gpu,ci" -- block --zkvm risc0 --action prove --resource gpu ${REPLAY_BLOCK_ARGS}

execute-sp1-ci:
	cargo r -r --features "sp1,ci" -- block --zkvm sp1 ${REPLAY_BLOCK_ARGS}
execute-risc0-ci:
	cargo r -r --no-default-features --features "risc0,ci" -- block --zkvm risc0 ${REPLAY_BLOCK_ARGS}

.PHONY: sp1 sp1-gpu risc0 risc0-gpu prove-sp1 prove-sp1-gpu \
		prove-sp1-gpu-ci prove-risc0 prove-risc0-gpu prove-risc0-gpu-ci \
		batch-sp1 batch-sp1-gpu batch-risc0 batch-risc0-gpu \
		prove-batch-sp1 prove-batch-sp1-gpu prove-batch-sp1-gpu-ci \
		prove-batch-risc0 prove-batch-risc0-gpu prove-batch-risc0-gpu-ci

NETWORK ?= mainnet

ifdef BLOCK_NUMBER
REPLAY_BLOCK_ARGS = ${BLOCK_NUMBER}
endif
REPLAY_BLOCK_ARGS += --rpc-url ${RPC_URL} --network ${NETWORK} --bench

ifdef BATCH_NUMBER
REPLAY_BATCH_ARGS = ${BATCH_NUMBER}
endif
REPLAY_BATCH_ARGS += --rpc-url ${RPC_URL} --network ${NETWORK} --bench


# Execution only
## Block
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- execute block ${REPLAY_BLOCK_ARGS}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- execute block ${REPLAY_BLOCK_ARGS}
risc0:
	cargo r -r --no-default-features --features risc0 -- execute block ${REPLAY_BLOCK_ARGS}
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- execute block ${REPLAY_BLOCK_ARGS}

## L2 Batch
batch-sp1:
	SP1_PROVER=cpu cargo r -r --features "sp1,l2" -- execute batch ${REPLAY_BATCH_ARGS}
batch-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,l2" -- execute batch ${REPLAY_BATCH_ARGS}
batch-risc0:
	cargo r -r --no-default-features --features "risc0,l2" -- execute batch ${REPLAY_BATCH_ARGS}
batch-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu,l2" -- execute batch ${REPLAY_BATCH_ARGS}

# Proving
## Block
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- prove block ${REPLAY_BLOCK_ARGS}
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- prove block ${REPLAY_BLOCK_ARGS}
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- prove block ${REPLAY_BLOCK_ARGS}
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- prove block ${REPLAY_BLOCK_ARGS}
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- prove block ${REPLAY_BLOCK_ARGS}
prove-risc0-gpu-ci:
	cargo r -r --no-default-features --features "risc0,gpu,ci" -- prove block ${REPLAY_BLOCK_ARGS}

## L2 Batch
prove-batch-sp1:
	SP1_PROVER=cpu cargo r -r --features "sp1,l2" -- prove batch ${REPLAY_BATCH_ARGS}
prove-batch-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,l2" -- prove batch ${REPLAY_BATCH_ARGS}
prove-batch-risc0:
	cargo r -r --no-default-features --features "risc0,l2" -- prove batch ${REPLAY_BATCH_ARGS}
prove-batch-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu,l2" -- prove batch ${REPLAY_BATCH_ARGS}

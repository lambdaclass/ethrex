.PHONY: sp1 sp1-gpu risc0 risc0-gpu prove-sp1 prove-sp1-gpu \
		prove-sp1-gpu-ci prove-risc0 prove-risc0-gpu prove-risc0-gpu-ci \
		batch-sp1 batch-sp1-gpu batch-risc0 batch-risc0-gpu \
		prove-batch-sp1 prove-batch-sp1-gpu \
		prove-batch-risc0 prove-batch-risc0-gpu

NETWORK ?= mainnet

# Block parameters
ifdef BLOCK_NUMBER
REPLAY_BLOCK_ARGS = ${BLOCK_NUMBER}
endif
REPLAY_BLOCK_ARGS += --rpc-url ${RPC_URL} --network ${NETWORK} --bench

# Batch parameters
REPLAY_BATCH_ARGS = ${BATCH_NUMBER} --rpc-url ${RPC_URL} --network ${NETWORK} --bench

# Transaction parameters
REPLAY_TX_ARGS = ${TX_HASH} --rpc-url ${RPC_URL} --network ${NETWORK}
ifdef L2
REPLAY_TX_ARGS += --l2
endif

# Block Composition parameters
REPLAY_BLOCK_COMPOSITION_ARGS = ${START_BLOCK} ${END_BLOCK} --rpc-url ${RPC_URL} --network ${NETWORK}

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

## Transaction
transaction:
	cargo r -r -- execute transaction ${REPLAY_TX_ARGS}

## Plot Composition
plot:
	cargo r -r -- block-composition ${REPLAY_BLOCK_COMPOSITION_ARGS}

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

help:
	cargo r -r -- --help

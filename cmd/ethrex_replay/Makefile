.PHONY: sp1 sp1-gpu risc0 risc0-gpu prove-sp1 prove-sp1-gpu \
		prove-sp1-gpu-ci prove-risc0 prove-risc0-gpu

NETWORK ?= mainnet
ifdef BLOCK_NUMBER
REPLAY_ARGS = ${BLOCK_NUMBER}
endif
REPLAY_ARGS += --rpc-url ${RPC_URL} --network ${NETWORK} --bench

# Execution only
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- execute block ${REPLAY_ARGS}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- execute block ${REPLAY_ARGS}
risc0:
	cargo r -r --no-default-features --features risc0 -- execute block ${REPLAY_ARGS}
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- execute block ${REPLAY_ARGS}

# Proving
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- prove block ${REPLAY_ARGS}
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- prove block ${REPLAY_ARGS}
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- prove block ${REPLAY_ARGS}
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- prove block ${REPLAY_ARGS}
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- prove block ${REPLAY_ARGS}
prove-risc0-gpu-ci:
	cargo r -r --no-default-features --features "risc0,gpu,ci" -- prove block ${REPLAY_ARGS}

.PHONY: sp1

NETWORK ?= mainnet
REPLAY_ARGS = --rpc-url ${RPC_URL} --network ${NETWORK} --bench

# Execution only
ifdef BLOCK_NUMBER
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- execute block ${BLOCK_NUMBER} ${REPLAY_ARGS}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- execute block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
risc0:
	cargo r -r --no-default-features --features risc0 -- execute block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- execute block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
pico:
	cargo +nightly r -r --features pico -- execute block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- execute block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
else
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- execute block ${REPLAY_ARGS}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- execute block ${REPLAY_ARGS}
risc0:
	cargo r -r --no-default-features --features risc0 -- execute block ${REPLAY_ARGS}
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- execute block ${REPLAY_ARGS}
pico:
	cargo +nightly r -r --features pico -- execute block ${REPLAY_ARGS}
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- execute block ${REPLAY_ARGS}
endif

# Proving
ifdef BLOCK_NUMBER
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
pico:
	cargo +nightly r -r --features pico -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- prove block ${BLOCK_NUMBER} ${REPLAY_ARGS} 
else
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- prove block $${REPLAY_ARGS}
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- prove block ${REPLAY_ARGS}
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- prove block ${REPLAY_ARGS}
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- prove block ${REPLAY_ARGS}
pico:
	cargo +nightly r -r --features pico -- prove block ${REPLAY_ARGS}
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- prove block ${REPLAY_ARGS}
endif

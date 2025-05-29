.PHONY: sp1

# Execution only
ifdef BLOCK_NUMBER
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- execute block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- execute block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
risc0:
	cargo r -r --no-default-features --features risc0 -- execute block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- execute block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
pico:
	cargo +nightly r -r --features pico -- execute block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- execute block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
else
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- execute block --rpc-url ${RPC_URL}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- execute block --rpc-url ${RPC_URL}
risc0:
	cargo r -r --no-default-features --features risc0 -- execute block --rpc-url ${RPC_URL}
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- execute block --rpc-url ${RPC_URL}
pico:
	cargo +nightly r -r --features pico -- execute block --rpc-url ${RPC_URL}
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- execute block --rpc-url ${RPC_URL}
endif

# Proving
ifdef BLOCK_NUMBER
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
pico:
	cargo +nightly r -r --features pico -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- prove block ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
else
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- prove block $--rpc-url ${RPC_URL}
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- prove block --rpc-url ${RPC_URL}
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- prove block --rpc-url ${RPC_URL}
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- prove block --rpc-url ${RPC_URL}
pico:
	cargo +nightly r -r --features pico -- prove block --rpc-url ${RPC_URL}
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- prove block --rpc-url ${RPC_URL}
endif

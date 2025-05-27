.PHONY: sp1

# Execution only
ifdef BLOCK_NUMBER
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- block execute ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- block execute ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
risc0:
	cargo r -r --no-default-features --features risc0 -- block execute ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- block execute ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
pico:
	cargo +nightly r -r --features pico -- block execute ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- block execute ${BLOCK_NUMBER} --rpc-url ${RPC_URL} 
else
sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- block execute --rpc-url ${RPC_URL}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- block execute --rpc-url ${RPC_URL}
risc0:
	cargo r -r --no-default-features --features risc0 -- block execute --rpc-url ${RPC_URL}
risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- block execute --rpc-url ${RPC_URL}
pico:
	cargo +nightly r -r --features pico -- block execute --rpc-url ${RPC_URL}
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- block execute --rpc-url ${RPC_URL}
endif

# Proving
ifdef BLOCK_NUMBER
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
prove-sp1-gpu-ci:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu,ci" -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
pico:
	cargo +nightly r -r --features pico -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- block prove ${BLOCK_NUMBER} --rpc-url ${RPC_URL} --prove 
else
prove-sp1:
	SP1_PROVER=cpu cargo r -r --features sp1 -- block prove $--rpc-url ${RPC_URL} --prove
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- block prove --rpc-url ${RPC_URL} --prove
prove-risc0:
	cargo r -r --no-default-features --features risc0 -- block prove --rpc-url ${RPC_URL} --prove
prove-risc0-gpu:
	cargo r -r --no-default-features --features "risc0,gpu" -- block prove --rpc-url ${RPC_URL} --prove
pico:
	cargo +nightly r -r --features pico -- block prove --rpc-url ${RPC_URL} --prove
pico-gpu:
	cargo +nightly r -r --features "pico,gpu" -- block prove --rpc-url ${RPC_URL} --prove
endif

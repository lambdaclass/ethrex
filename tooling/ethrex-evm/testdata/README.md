# Test data

Inputs that the execution-specs filler sent to the t8n, with what the EELS t8n answers to them. The tests in `src/t8n/tests.rs` run `ethrex-evm t8n` on the inputs and compare.

## `blockhash_two_blocks_back`

The t8n call for block 3 of `test_witness_headers_blockhash_at_offset[fork_Amsterdam-blockchain_test-offset_2]`, in `tests/amsterdam/eip8025_optional_proofs/test_witness_headers.py` of execution-specs (branch `projects/zkevm`, commit `1662ffe3e9`). The only transaction of the block calls a contract that stores `BLOCKHASH(1)`, so the witness has the headers of blocks 1 and 2.

- `alloc.json`, `env.json` and `txs.json` are the input, as the filler dumps it with `--evm-dump-dir`.
- `eels_witness.json` is the `executionWitness` of the result of EELS.
- `eels_opcode_counts.json` is what EELS writes with `--opcode.count`, with the keys sorted.

To get them again, fill the test with EELS and take the input and the result of the third call:

    uv run fill --fork=Amsterdam -m blockchain_test -k "blockhash_at_offset and offset_2" \
        tests/amsterdam/eip8025_optional_proofs/test_witness_headers.py --evm-dump-dir=<dump>

The fill doesn't count opcodes for this test, so the counts come from running the EELS t8n on the dumped input:

    uv run ethereum-spec-evm t8n --state.fork Amsterdam --state.chainid 1 --state.reward 0 \
        --input.alloc alloc.json --input.env env.json --input.txs txs.json \
        --output.basedir <out> --opcode.count opcodes.json

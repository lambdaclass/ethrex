# Test data

These are inputs that the execution-specs filler sent to the t8n, with what the EELS t8n answers to them. The tests in `src/t8n/tests.rs` run `ethrex-evm t8n` on the inputs and compare.

## `blockhash_two_blocks_back`

This is the t8n call for block 3 of `test_witness_headers_blockhash_at_offset[fork_Amsterdam-blockchain_test-offset_2]`, in `tests/amsterdam/eip8025_optional_proofs/test_witness_headers.py` of execution-specs (branch `projects/zkevm`, commit `1662ffe3e9`). The only transaction of the block calls a contract that stores `BLOCKHASH(1)`, so the witness has the headers of blocks 1 and 2.

- `alloc.json`, `env.json` and `txs.json` are the input, as the filler dumps it with `--evm-dump-dir`.
- `eels_witness.json` is the `executionWitness` in the result of EELS.
- `eels_opcode_counts.json` is what EELS writes with `--opcode.count`, with the keys sorted.

To get them again, fill the test with EELS and take the input and the result of the third call:

    uv run fill --fork=Amsterdam -m blockchain_test -k "blockhash_at_offset and offset_2" \
        tests/amsterdam/eip8025_optional_proofs/test_witness_headers.py --evm-dump-dir=<dump>

The fill doesn't count opcodes for this test, so the counts come from running the EELS t8n on the dumped input:

    uv run ethereum-spec-evm t8n --state.fork Amsterdam --state.chainid 1 --state.reward 0 \
        --input.alloc alloc.json --input.env env.json --input.txs txs.json \
        --output.basedir <out> --opcode.count opcodes.json

## `account_deleted_then_another_inserted`

This is the only t8n call of `test_witness_state_account_delete_uses_insert_before_delete_order[fork_Amsterdam-blockchain_test-delete_then_insert]`, in `tests/amsterdam/eip8025_optional_proofs/test_witness_state_replay_order.py` of execution-specs (branch `projects/zkevm`, commit `fd98fc88bf`). The first transaction deploys a contract that self-destructs right away to an address that has balance in the pre-state, so the block deletes that account. The second one funds a new account that shares a branch of the state trie with the deleted one.

- `alloc.json`, `env.json` and `txs.json` are the input, as the filler dumps it with `--evm-dump-dir`.
- `eels_alloc.json` is the output alloc of EELS, which doesn't have the deleted account.
- `eels_witness.json` is the `executionWitness` in the result of EELS. It doesn't have the leaf of the account under that branch that the block doesn't touch, since EELS inserts before it deletes.

To get them again, fill the test with EELS and take the input and the output of the call:

    uv run fill --fork=Amsterdam -m blockchain_test -k "account_delete_uses_insert_before_delete_order and delete_then_insert" \
        tests/amsterdam/eip8025_optional_proofs/test_witness_state_replay_order.py --evm-dump-dir=<dump>

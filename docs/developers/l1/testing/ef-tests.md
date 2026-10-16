# Ethereum foundation tests

These are the official execution spec tests. There are two kinds: `state tests` and `blockchain tests`, you can execute them with:

### State tests

The state tests are individual transactions not related one to each other that test particular behavior of the EVM. Tests are usually run for multiple forks and the result of execution may vary between forks.
See [docs](https://steel.ethereum.foundation/docs/execution-specs/running_tests/test_formats/state_test/).

To run the test first:

```sh
cd tooling/ef_tests/state
```

then download the test vectors:

```sh
make download-evm-ef-tests
```

then run the tests:

```sh
make run-evm-ef-tests
```

### Blockchain tests


The blockchain tests test block validation and the consensus rules of the Ethereum blockchain. Tests are usually run for multiple forks.
See [docs](https://steel.ethereum.foundation/docs/execution-specs/running_tests/test_formats/blockchain_test/).

To run the tests first:

```sh
cd tooling/ef_tests/blockchain
```

then run the tests:

```sh
make test-levm
```

### Standalone runners

`blocktest` and `enginetest` run fixtures from any directory and print one result per fixture, which is how EELS `consume direct` calls them. From `tooling/`:

```sh
cargo run --profile release-fast -p ef_tests-runners --bin blocktest -- <path>... [--workers N] [--json] [--bal-sequential] [--bal-withhold] [--no-precompile-cache] [--bal-report]
cargo run --profile release-fast -p ef_tests-runners --bin enginetest -- <path>... [same flags]
```

- Each `<path>` is a fixture file or a directory searched recursively for `.json` files; all of them run in one process and print one set of results. Both runners exit 0 when every fixture ran and passed, 1 when every fixture ran and at least one failed, and 2 when they could not run all of their input: a path that does not exist, or a file or directory that cannot be read, whether named or found under a directory (reported on stderr before any fixture runs), or a fixture file that does not parse (reported on stderr and left out of the results; the other files still run and are reported).
- `blocktest` imports each `blockchain_test` block through the blockchain ef_tests harness, handing it the fixture's `blockAccessList` (or `rlp_decoded.blockAccessList` for a block expected to be invalid) when the keccak of its RLP, in the order given, equals `blockAccessListHash`. A list that does not match or does not parse is dropped, as full sync drops a peer's, and the block runs without it.
- `enginetest` sends each `blockchain_test_engine` payload and forkchoice update to ethrex's own `engine_newPayloadV<n>` and `engine_forkchoiceUpdatedV<n>` handlers, in process. After the last payload, the chain's head must be the fixture's `lastblockhash`.
- Each `blocktest` and `enginetest` result lists the blocks or payloads ethrex rejected in `rejections`, `[]` when none: `{"index":0,"hash":"0x…","error":"…"}`, where `index` is the position in `blocks` or `engineNewPayloads` and `error` is ethrex's error as given. A block that does not decode has the decoder's error and no hash; a payload rejected with a JSON-RPC error has `<code>: <message>` (plus `: <data>` when the error has data) and no hash. The runners never compare these errors with the fixture's expected exception: a fixture passes when what it expects to be rejected is rejected, whatever the reason, and a harness reading the results can check the reasons with EEST's mapping. `enginetest` does compare a JSON-RPC error's code with the payload's `errorCode`. `ETHREX_ENGINE_STRICT_EXCEPTIONS` applies to the cargo engine harness only.
- ethrex's `statetest` is `ef_tests-statev2 statetest` (see `tooling/ef_tests/state_v2`).
- `--version` prints one line naming the client and the tool, `ethrex-blocktest <version>` or `ethrex-enginetest <version>`, where the version is the tooling workspace's; harnesses identify a runner by it.
- `--bal-sequential` runs every block on the sequential executor, like the node's `--no-bal-parallel-exec`.
- `--no-precompile-cache` (or `ETHREX_NO_PRECOMPILE_CACHE`) runs every block without the per-block precompile result cache, like the node flag of the same name.
- `blocktest --bal-withhold` never delivers the fixture's access list, so every block runs without one and ethrex checks the list it builds against the header, as for a block received without its list.
- With `--bal-report`, `blocktest` and `enginetest` print the executor chosen for each block as one JSON line on stderr, for example `{"event":"balExecution","block":1,"hash":"0x…","path":"sequential","reason":"disabled"}`. `reason` is `bad-access-list` when the runner dropped the block's delivered list and `withheld` when `--bal-withhold` kept back the list the block has, whatever ran the block; a block without a list keeps its own reason under `--bal-withhold`. Otherwise it is empty for `parallel`, and for `sequential` the first condition that ruled parallel out: `no-access-list`, `pre-amsterdam`, `disabled` or `no-rayon`. A list over the EIP-7928 item cap shows as `no-access-list`, because the node drops it before the executor. Without the flag these lines are off. stdout carries only the results.

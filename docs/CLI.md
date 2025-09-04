# CLI Commands

## ethrex

<!-- BEGIN_CLI_HELP -->

```
ethrex Execution client

Usage: ethrex [OPTIONS] [COMMAND]

Commands:
  removedb            Remove the database
  import              Import blocks to the database
  export              Export blocks in the current chain into a file in rlp encoding
  compute-state-root  Compute the state root from a genesis file
  l2
  help                Print this message or the help of the given subcommand(s)

Options:
  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

Node options:
      --network <GENESIS_FILE_PATH>
          Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia, hoodi and mainnet. If not specified, defaults to mainnet.

          [env: ETHREX_NETWORK=]

      --datadir <DATABASE_DIRECTORY>
          If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.

          [env: ETHREX_DATADIR=]
          [default: /home/runner/.local/share/ethrex]

      --force
          Delete the database without confirmation.

      --metrics.addr <ADDRESS>
          [default: 0.0.0.0]

      --metrics.port <PROMETHEUS_METRICS_PORT>
          [env: ETHREX_METRICS_PORT=]
          [default: 9090]

      --metrics
          Enable metrics collection and exposition

      --dev
          If set it will be considered as `true`. If `--network` is not specified, it will default to a custom local devnet. The Binary has to be built with the `dev` feature enabled.

      --evm <EVM_BACKEND>
          Has to be `levm` or `revm`

          [env: ETHREX_EVM=]
          [default: levm]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [default: INFO]

P2P options:
      --bootnodes <BOOTNODE_LIST>...
          Comma separated enode URLs for P2P discovery bootstrap.

      --syncmode <SYNC_MODE>
          Can be either "full" or "snap" with "full" as default value.

          [default: full]

      --p2p.enabled


      --p2p.addr <ADDRESS>
          [default: 0.0.0.0]

      --p2p.port <PORT>
          [default: 30303]

      --discovery.addr <ADDRESS>
          UDP address for P2P discovery.

          [default: 0.0.0.0]

      --discovery.port <PORT>
          UDP port for P2P discovery.

          [default: 30303]

RPC options:
      --http.addr <ADDRESS>
          Listening address for the http rpc server.

          [env: ETHREX_HTTP_ADDR=]
          [default: localhost]

      --http.port <PORT>
          Listening port for the http rpc server.

          [env: ETHREX_HTTP_PORT=]
          [default: 8545]

      --authrpc.addr <ADDRESS>
          Listening address for the authenticated rpc server.

          [default: localhost]

      --authrpc.port <PORT>
          Listening port for the authenticated rpc server.

          [default: 8551]

      --authrpc.jwtsecret <JWTSECRET_PATH>
          Receives the jwt secret used for authenticated rpc requests.

          [default: jwt.hex]
```

<!-- END_CLI_HELP -->

## ethrex l2

```
Usage: ethrex l2 [OPTIONS]
       ethrex l2 <COMMAND>

Commands:
  prover        Initialize an ethrex prover [aliases: p]
  removedb      Remove the database [aliases: rm, clean]
  blobs-saver   Launch a server that listens for Blobs submissions and saves them offline.
  reconstruct   Reconstructs the L2 state from L1 blobs.
  revert-batch  Reverts unverified batches.
  deploy        Deploy in L1 all contracts needed by an L2.
  help          Print this message or the help of the given subcommand(s)

Options:
  -t, --tick-rate <TICK_RATE>
          time in ms between two ticks

          [default: 1000]

      --batch-widget-height <BATCH_WIDGET_HEIGHT>


  -h, --help
          Print help (see a summary with '-h')

Node options:
      --network <GENESIS_FILE_PATH>
          Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia, hoodi and mainnet. If not specified, defaults to mainnet.

          [env: ETHREX_NETWORK=]

      --datadir <DATABASE_DIRECTORY>
          If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.

          [env: ETHREX_DATADIR=]
          [default: /home/runner/.local/share/ethrex]

      --force
          Delete the database without confirmation.

      --metrics.addr <ADDRESS>
          [default: 0.0.0.0]

      --metrics.port <PROMETHEUS_METRICS_PORT>
          [env: ETHREX_METRICS_PORT=]
          [default: 9090]

      --metrics
          Enable metrics collection and exposition

      --dev
          If set it will be considered as `true`. If `--network` is not specified, it will default to a custom local devnet. The Binary has to be built with the `dev` feature enabled.

      --evm <EVM_BACKEND>
          Has to be `levm` or `revm`

          [env: ETHREX_EVM=]
          [default: levm]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [default: INFO]

P2P options:
      --bootnodes <BOOTNODE_LIST>...
          Comma separated enode URLs for P2P discovery bootstrap.

      --syncmode <SYNC_MODE>
          Can be either "full" or "snap" with "full" as default value.

          [default: full]

      --p2p.enabled


      --p2p.addr <ADDRESS>
          [default: 0.0.0.0]

      --p2p.port <PORT>
          [default: 30303]

      --discovery.addr <ADDRESS>
          UDP address for P2P discovery.

          [default: 0.0.0.0]

      --discovery.port <PORT>
          UDP port for P2P discovery.

          [default: 30303]

RPC options:
      --http.addr <ADDRESS>
          Listening address for the http rpc server.

          [env: ETHREX_HTTP_ADDR=]
          [default: localhost]

      --http.port <PORT>
          Listening port for the http rpc server.

          [env: ETHREX_HTTP_PORT=]
          [default: 8545]

      --authrpc.addr <ADDRESS>
          Listening address for the authenticated rpc server.

          [default: localhost]

      --authrpc.port <PORT>
          Listening port for the authenticated rpc server.

          [default: 8551]

      --authrpc.jwtsecret <JWTSECRET_PATH>
          Receives the jwt secret used for authenticated rpc requests.

          [default: jwt.hex]

Eth options:
      --eth.rpc-url <RPC_URL>...
          List of rpc urls to use.

          [env: ETHREX_ETH_RPC_URL=]

      --eth.maximum-allowed-max-fee-per-gas <UINT64>
          [env: ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_GAS=]
          [default: 10000000000]

      --eth.maximum-allowed-max-fee-per-blob-gas <UINT64>
          [env: ETHREX_MAXIMUM_ALLOWED_MAX_FEE_PER_BLOB_GAS=]
          [default: 10000000000]

      --eth.max-number-of-retries <UINT64>
          [env: ETHREX_MAX_NUMBER_OF_RETRIES=]
          [default: 10]

      --eth.backoff-factor <UINT64>
          [env: ETHREX_BACKOFF_FACTOR=]
          [default: 2]

      --eth.min-retry-delay <UINT64>
          [env: ETHREX_MIN_RETRY_DELAY=]
          [default: 96]

      --eth.max-retry-delay <UINT64>
          [env: ETHREX_MAX_RETRY_DELAY=]
          [default: 1800]

L1 Watcher options:
      --l1.bridge-address <ADDRESS>
          [env: ETHREX_WATCHER_BRIDGE_ADDRESS=]

      --watcher.watch-interval <UINT64>
          How often the L1 watcher checks for new blocks in milliseconds.

          [env: ETHREX_WATCHER_WATCH_INTERVAL=]
          [default: 1000]

      --watcher.max-block-step <UINT64>
          [env: ETHREX_WATCHER_MAX_BLOCK_STEP=]
          [default: 5000]

      --watcher.block-delay <UINT64>
          Number of blocks the L1 watcher waits before trusting an L1 block.

          [env: ETHREX_WATCHER_BLOCK_DELAY=]
          [default: 10]

Block producer options:
      --block-producer.block-time <UINT64>
          How often does the sequencer produce new blocks to the L1 in milliseconds.

          [env: ETHREX_BLOCK_PRODUCER_BLOCK_TIME=]
          [default: 5000]

      --block-producer.coinbase-address <ADDRESS>
          [env: ETHREX_BLOCK_PRODUCER_COINBASE_ADDRESS=]

Proposer options:
      --elasticity-multiplier <UINT64>
          [env: ETHREX_PROPOSER_ELASTICITY_MULTIPLIER=]
          [default: 2]

L1 Committer options:
      --committer.l1-private-key <PRIVATE_KEY>
          Private key of a funded account that the sequencer will use to send commit txs to the L1.

          [env: ETHREX_COMMITTER_L1_PRIVATE_KEY=]

      --committer.remote-signer-url <URL>
          URL of a Web3Signer-compatible server to remote sign instead of a local private key.

          [env: ETHREX_COMMITTER_REMOTE_SIGNER_URL=]

      --committer.remote-signer-public-key <PUBLIC_KEY>
          Public key to request the remote signature from.

          [env: ETHREX_COMMITTER_REMOTE_SIGNER_PUBLIC_KEY=]

      --l1.on-chain-proposer-address <ADDRESS>
          [env: ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=]

      --committer.commit-time <UINT64>
          How often does the sequencer commit new blocks to the L1 in milliseconds.

          [env: ETHREX_COMMITTER_COMMIT_TIME=]
          [default: 60000]

      --committer.arbitrary-base-blob-gas-price <UINT64>
          [env: ETHREX_COMMITTER_ARBITRARY_BASE_BLOB_GAS_PRICE=]
          [default: 1000000000]

Proof coordinator options:
      --proof-coordinator.l1-private-key <PRIVATE_KEY>
          Private key of of a funded account that the sequencer will use to send verify txs to the L1. Has to be a different account than --committer-l1-private-key.

          [env: ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=]

      --proof-coordinator.tdx-private-key <PRIVATE_KEY>
          Private key of of a funded account that the TDX tool that will use to send the tdx attestation to L1.

          [env: ETHREX_PROOF_COORDINATOR_TDX_PRIVATE_KEY=]

      --proof-coordinator.remote-signer-url <URL>
          URL of a Web3Signer-compatible server to remote sign instead of a local private key.

          [env: ETHREX_PROOF_COORDINATOR_REMOTE_SIGNER_URL=]

      --proof-coordinator.remote-signer-public-key <PUBLIC_KEY>
          Public key to request the remote signature from.

          [env: ETHREX_PROOF_COORDINATOR_REMOTE_SIGNER_PUBLIC_KEY=]

      --proof-coordinator.addr <IP_ADDRESS>
          Set it to 0.0.0.0 to allow connections from other machines.

          [env: ETHREX_PROOF_COORDINATOR_LISTEN_ADDRESS=]
          [default: 127.0.0.1]

      --proof-coordinator.port <UINT16>
          [env: ETHREX_PROOF_COORDINATOR_LISTEN_PORT=]
          [default: 3900]

      --proof-coordinator.send-interval <UINT64>
          How often does the proof coordinator send proofs to the L1 in milliseconds.

          [env: ETHREX_PROOF_COORDINATOR_SEND_INTERVAL=]
          [default: 5000]

      --proof-coordinator.dev-mode
          [env: ETHREX_PROOF_COORDINATOR_DEV_MODE=]

Based options:
      --state-updater.sequencer-registry <ADDRESS>
          [env: ETHREX_STATE_UPDATER_SEQUENCER_REGISTRY=]

      --state-updater.check-interval <UINT64>
          [env: ETHREX_STATE_UPDATER_CHECK_INTERVAL=]
          [default: 1000]

      --block-fetcher.fetch_interval_ms <UINT64>
          [env: ETHREX_BLOCK_FETCHER_FETCH_INTERVAL_MS=]
          [default: 5000]

      --fetch-block-step <UINT64>
          [env: ETHREX_BLOCK_FETCHER_FETCH_BLOCK_STEP=]
          [default: 5000]

      --based
          [env: ETHREX_BASED=]

Aligned options:
      --aligned
          [env: ETHREX_ALIGNED_MODE=]

      --aligned-verifier-interval-ms <ETHREX_ALIGNED_VERIFIER_INTERVAL_MS>
          [env: ETHREX_ALIGNED_VERIFIER_INTERVAL_MS=]
          [default: 5000]

      --aligned.beacon-url <BEACON_URL>...
          List of beacon urls to use.

          [env: ETHREX_ALIGNED_BEACON_URL=]

      --aligned-network <ETHREX_ALIGNED_NETWORK>
          L1 network name for Aligned sdk

          [env: ETHREX_ALIGNED_NETWORK=]
          [default: devnet]

      --aligned.fee-estimate <FEE_ESTIMATE>
          Fee estimate for Aligned sdk

          [env: ETHREX_ALIGNED_FEE_ESTIMATE=]
          [default: instant]

      --aligned-sp1-elf-path <ETHREX_ALIGNED_SP1_ELF_PATH>
          Path to the SP1 elf. This is used for proof verification.

          [env: ETHREX_ALIGNED_SP1_ELF_PATH=]

L2 options:
      --validium
          If true, L2 will run on validium mode as opposed to the default rollup mode, meaning it will not publish state diffs to the L1.

          [env: ETHREX_L2_VALIDIUM=]

      --sponsorable-addresses <SPONSORABLE_ADDRESSES_PATH>
          Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs

      --sponsor-private-key <SPONSOR_PRIVATE_KEY>
          The private key of ethrex L2 transactions sponsor.

          [env: SPONSOR_PRIVATE_KEY=]
          [default: 0xffd790338a2798b648806fc8635ac7bf14af15425fed0c8f25bcc5febaa9b192]

Monitor options:
      --no-monitor
          [env: ETHREX_NO_MONITOR=]
```

## ethrex l2 prover

```
Initialize an ethrex prover

Usage: ethrex l2 prover [OPTIONS] --proof-coordinators <URL>

Options:
  -h, --help
          Print help (see a summary with '-h')

Prover client options:
      --backend <BACKEND>
          [env: PROVER_CLIENT_BACKEND=]
          [default: exec]
          [possible values: exec]

      --proof-coordinators <URL>
          URL of the sequencer's proof coordinator

          [env: PROVER_CLIENT_PROOF_COORDINATOR_URL=]

      --proving-time <PROVING_TIME>
          Time to wait before requesting new data to prove

          [env: PROVER_CLIENT_PROVING_TIME=]
          [default: 5000]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [default: INFO]

      --aligned
          Activate aligned proving system

          [env: PROVER_CLIENT_ALIGNED=]
```

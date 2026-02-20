# CLI Commands

## ethrex

<!-- BEGIN_CLI_HELP -->

```
ethrex Execution client

Usage: ethrex [OPTIONS] [COMMAND]

Commands:
  removedb            Remove the database
  import              Import blocks to the database
  import-bench        Import blocks to the database for benchmarking
  export              Export blocks in the current chain into a file in rlp encoding
  compute-state-root  Compute the state root from a genesis file
  repl                Interactive REPL for Ethereum JSON-RPC
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
          [env: ETHREX_METRICS_ADDR=]
          [default: 0.0.0.0]

      --metrics.port <PROMETHEUS_METRICS_PORT>
          [env: ETHREX_METRICS_PORT=]
          [default: 9090]

      --metrics
          Enable metrics collection and exposition

          [env: ETHREX_METRICS=]

      --dev
          If set it will be considered as `true`. If `--network` is not specified, it will default to a custom local devnet. The Binary has to be built with the `dev` feature enabled.

          [env: ETHREX_DEV=]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [env: ETHREX_LOG_LEVEL=]
          [default: INFO]

      --log.color <LOG_COLOR>
          Possible values: auto, always, never

          [env: ETHREX_LOG_COLOR=]
          [default: auto]

      --log.dir <LOG_DIR>
          Directory to store log files.

          [env: ETHREX_LOG_DIR=]

      --mempool.maxsize <MEMPOOL_MAX_SIZE>
          Maximum size of the mempool in number of transactions

          [env: ETHREX_MEMPOOL_MAXSIZE=]
          [default: 10000]

      --precompute-witnesses
          Once synced, computes execution witnesses upon receiving newPayload messages and stores them in local storage

          [env: ETHREX_PRECOMPUTE_WITNESSES=]

P2P options:
      --bootnodes <BOOTNODE_LIST>...
          Comma separated enode URLs for P2P discovery bootstrap.

          [env: ETHREX_BOOTNODES=]

      --syncmode <SYNC_MODE>
          Can be either "full" or "snap" with "snap" as default value.

          [env: ETHREX_SYNCMODE=]
          [default: snap]

      --p2p.disabled
          [env: ETHREX_P2P_DISABLED=]

      --p2p.addr <ADDRESS>
          Listening address for the P2P protocol.

          [env: ETHREX_P2P_ADDR=]

      --p2p.port <PORT>
          TCP port for the P2P protocol.

          [env: ETHREX_P2P_PORT=]
          [default: 30303]

      --discovery.port <PORT>
          UDP port for P2P discovery.

          [env: ETHREX_DISCOVERY_PORT=]
          [default: 30303]

      --p2p.tx-broadcasting-interval <INTERVAL_MS>
          Transaction Broadcasting Time Interval (ms) for batching transactions before broadcasting them.

          [env: ETHREX_P2P_TX_BROADCASTING_INTERVAL=]
          [default: 1000]

      --p2p.target-peers <MAX_PEERS>
          Max amount of connected peers.

          [env: ETHREX_P2P_TARGET_PEERS=]
          [default: 100]

      --p2p.lookup-interval <INITIAL_LOOKUP_INTERVAL>
          Initial Lookup Time Interval (ms) to trigger each Discovery lookup message and RLPx connection attempt.

          [env: ETHREX_P2P_LOOKUP_INTERVAL=]
          [default: 100]

RPC options:
      --http.addr <ADDRESS>
          Listening address for the http rpc server.

          [env: ETHREX_HTTP_ADDR=]
          [default: 0.0.0.0]

      --http.port <PORT>
          Listening port for the http rpc server.

          [env: ETHREX_HTTP_PORT=]
          [default: 8545]

      --ws.enabled
          Enable websocket rpc server. Disabled by default.

          [env: ETHREX_ENABLE_WS=]

      --ws.addr <ADDRESS>
          Listening address for the websocket rpc server.

          [env: ETHREX_WS_ADDR=]
          [default: 0.0.0.0]

      --ws.port <PORT>
          Listening port for the websocket rpc server.

          [env: ETHREX_WS_PORT=]
          [default: 8546]

      --authrpc.addr <ADDRESS>
          Listening address for the authenticated rpc server.

          [env: ETHREX_AUTHRPC_ADDR=]
          [default: 127.0.0.1]

      --authrpc.port <PORT>
          Listening port for the authenticated rpc server.

          [env: ETHREX_AUTHRPC_PORT=]
          [default: 8551]

      --authrpc.jwtsecret <JWTSECRET_PATH>
          Receives the jwt secret used for authenticated rpc requests.

          [env: ETHREX_AUTHRPC_JWTSECRET_PATH=]
          [default: jwt.hex]

Block building options:
      --builder.extra-data <EXTRA_DATA>
          Block extra data message.

          [env: ETHREX_BUILDER_EXTRA_DATA=]
          [default: "ethrex 9.0.0"]

      --builder.gas-limit <GAS_LIMIT>
          Target block gas limit.

          [env: ETHREX_BUILDER_GAS_LIMIT=]
          [default: 60000000]

      --builder.max-blobs <MAX_BLOBS>
          EIP-7872: Maximum blobs per block for local building. Minimum of 1. Defaults to protocol max.

          [env: ETHREX_BUILDER_MAX_BLOBS=]
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
  pause         Pause L1 contracts
  unpause       Unpause L1 contracts
  deploy        Deploy in L1 all contracts needed by an L2.
  help          Print this message or the help of the given subcommand(s)

Options:
      --osaka-activation-time <UINT64>
          Block timestamp at which the Osaka fork is activated on L1. If not set, it will assume Osaka is already active.

          [env: ETHREX_OSAKA_ACTIVATION_TIME=]

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
          [default: "/home/runner/.local/share/ethrex"]

      --force
          Delete the database without confirmation.

      --metrics.addr <ADDRESS>
          [env: ETHREX_METRICS_ADDR=]
          [default: 0.0.0.0]

      --metrics.port <PROMETHEUS_METRICS_PORT>
          [env: ETHREX_METRICS_PORT=]
          [default: 9090]

      --metrics
          Enable metrics collection and exposition

          [env: ETHREX_METRICS=]

      --dev
          If set it will be considered as `true`. If `--network` is not specified, it will default to a custom local devnet. The Binary has to be built with the `dev` feature enabled.

          [env: ETHREX_DEV=]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [env: ETHREX_LOG_LEVEL=]
          [default: INFO]

      --log.color <LOG_COLOR>
          Possible values: auto, always, never

          [env: ETHREX_LOG_COLOR=]
          [default: auto]

      --mempool.maxsize <MEMPOOL_MAX_SIZE>
          Maximum size of the mempool in number of transactions

          [env: ETHREX_MEMPOOL_MAXSIZE=]
          [default: 10000]

P2P options:
      --bootnodes <BOOTNODE_LIST>...
          Comma separated enode URLs for P2P discovery bootstrap.

          [env: ETHREX_BOOTNODES=]

      --syncmode <SYNC_MODE>
          Can be either "full" or "snap" with "snap" as default value.

          [env: ETHREX_SYNCMODE=]
          [default: snap]

      --p2p.disabled

          [env: ETHREX_P2P_DISABLED=]

      --p2p.addr <ADDRESS>
          Listening address for the P2P protocol.

          [env: ETHREX_P2P_ADDR=]

      --p2p.port <PORT>
          TCP port for the P2P protocol.

          [env: ETHREX_P2P_PORT=]
          [default: 30303]

      --discovery.port <PORT>
          UDP port for P2P discovery.

          [env: ETHREX_DISCOVERY_PORT=]
          [default: 30303]

      --p2p.tx-broadcasting-interval <INTERVAL_MS>
          Transaction Broadcasting Time Interval (ms) for batching transactions before broadcasting them.

          [env: ETHREX_P2P_TX_BROADCASTING_INTERVAL=]
          [default: 1000]

      --target.peers <MAX_PEERS>
          Max amount of connected peers.

          [env: ETHREX_P2P_TARGET_PEERS=]
          [default: 100]

RPC options:
      --http.addr <ADDRESS>
          Listening address for the http rpc server.

          [env: ETHREX_HTTP_ADDR=]
          [default: 0.0.0.0]

      --http.port <PORT>
          Listening port for the http rpc server.

          [env: ETHREX_HTTP_PORT=]
          [default: 8545]

      --ws.enabled
          Enable websocket rpc server. Disabled by default.

          [env: ETHREX_ENABLE_WS=]

      --ws.addr <ADDRESS>
          Listening address for the websocket rpc server.

          [env: ETHREX_WS_ADDR=]
          [default: 0.0.0.0]

      --ws.port <PORT>
          Listening port for the websocket rpc server.

          [env: ETHREX_WS_PORT=]
          [default: 8546]

      --authrpc.addr <ADDRESS>
          Listening address for the authenticated rpc server.

          [env: ETHREX_AUTHRPC_ADDR=]
          [default: 127.0.0.1]

      --authrpc.port <PORT>
          Listening port for the authenticated rpc server.

          [env: ETHREX_AUTHRPC_PORT=]
          [default: 8551]

      --authrpc.jwtsecret <JWTSECRET_PATH>
          Receives the jwt secret used for authenticated rpc requests.

          [env: ETHREX_AUTHRPC_JWTSECRET_PATH=]
          [default: jwt.hex]

Block building options:
      --builder.extra-data <EXTRA_DATA>
          Block extra data message.

          [env: ETHREX_BUILDER_EXTRA_DATA=]
          [default: "ethrex 9.0.0"]

      --builder.gas-limit <GAS_LIMIT>
          Target block gas limit.

          [env: ETHREX_BUILDER_GAS_LIMIT=]
          [default: 60000000]

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
          [default: 12000]

      --watcher.max-block-step <UINT64>
          [env: ETHREX_WATCHER_MAX_BLOCK_STEP=]
          [default: 5000]

      --watcher.block-delay <UINT64>
          Number of blocks the L1 watcher waits before trusting an L1 block.

          [env: ETHREX_WATCHER_BLOCK_DELAY=]
          [default: 10]

Block producer options:
      --watcher.l1-fee-update-interval-ms <ADDRESS>
          [env: ETHREX_WATCHER_L1_FEE_UPDATE_INTERVAL_MS=]
          [default: 60000]

      --block-producer.block-time <UINT64>
          How often does the sequencer produce new blocks to the L1 in milliseconds.

          [env: ETHREX_BLOCK_PRODUCER_BLOCK_TIME=]
          [default: 5000]

      --block-producer.coinbase-address <ADDRESS>
          [env: ETHREX_BLOCK_PRODUCER_COINBASE_ADDRESS=]

      --block-producer.base-fee-vault-address <ADDRESS>
          [env: ETHREX_BLOCK_PRODUCER_BASE_FEE_VAULT_ADDRESS=]

      --block-producer.operator-fee-vault-address <ADDRESS>
          [env: ETHREX_BLOCK_PRODUCER_OPERATOR_FEE_VAULT_ADDRESS=]

      --block-producer.operator-fee-per-gas <UINT64>
          Fee that the operator will receive for each unit of gas consumed in a block.

          [env: ETHREX_BLOCK_PRODUCER_OPERATOR_FEE_PER_GAS=]

      --block-producer.l1-fee-vault-address <ADDRESS>
          [env: ETHREX_BLOCK_PRODUCER_L1_FEE_VAULT_ADDRESS=]

      --block-producer.block-gas-limit <UINT64>
          Maximum gas limit for the L2 blocks.

          [env: ETHREX_BLOCK_PRODUCER_BLOCK_GAS_LIMIT=]
          [default: 30000000]

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

      --committer.batch-gas-limit <UINT64>
          Maximum gas limit for the batch

          [env: ETHREX_COMMITTER_BATCH_GAS_LIMIT=]

      --committer.first-wake-up-time <UINT64>
          Time to wait before the sequencer seals a batch when started. After committing the first batch, `committer.commit-time` will be used.

          [env: ETHREX_COMMITTER_FIRST_WAKE_UP_TIME=]

      --committer.arbitrary-base-blob-gas-price <UINT64>
          [env: ETHREX_COMMITTER_ARBITRARY_BASE_BLOB_GAS_PRICE=]
          [default: 1000000000]

Proof coordinator options:
      --proof-coordinator.l1-private-key <PRIVATE_KEY>
          Private key of a funded account that the sequencer will use to send verify txs to the L1. Has to be a different account than --committer-l1-private-key.

          [env: ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=]

      --proof-coordinator.tdx-private-key <PRIVATE_KEY>
          Private key of a funded account that the TDX tool will use to send the tdx attestation to L1.

          [env: ETHREX_PROOF_COORDINATOR_TDX_PRIVATE_KEY=]

      --proof-coordinator.qpl-tool-path <QPL_TOOL_PATH>
          Path to the QPL tool that will be used to generate TDX quotes.

          [env: ETHREX_PROOF_COORDINATOR_QPL_TOOL_PATH=]
          [default: ./tee/contracts/automata-dcap-qpl/automata-dcap-qpl-tool/target/release/automata-dcap-qpl-tool]

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

      --aligned.from-block <BLOCK_NUMBER>
          Starting L1 block number for proof aggregation search. Helps avoid scanning blocks from before proofs were being sent.

          [env: ETHREX_ALIGNED_FROM_BLOCK=]

      --aligned.fee-estimate <FEE_ESTIMATE>
          Fee estimate for Aligned sdk

          [env: ETHREX_ALIGNED_FEE_ESTIMATE=]
          [default: instant]

Admin server options:
      --admin-server.addr <IP_ADDRESS>
          [env: ETHREX_ADMIN_SERVER_LISTEN_ADDRESS=]
          [default: 127.0.0.1]

      --admin-server.port <UINT16>
          [env: ETHREX_ADMIN_SERVER_LISTEN_PORT=]
          [default: 5555]

L2 options:
      --validium
          If true, L2 will run on validium mode as opposed to the default rollup mode, meaning it will not publish blobs to the L1.

          [env: ETHREX_L2_VALIDIUM=]

      --sponsorable-addresses <SPONSORABLE_ADDRESSES_PATH>
          Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs

          [env: ETHREX_SPONSORABLE_ADDRESSES_PATH=]

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

Usage: ethrex l2 prover [OPTIONS] --proof-coordinators <URL>...

Options:
  -h, --help
          Print help (see a summary with '-h')

Prover client options:
      --backend <BACKEND>
          [env: PROVER_CLIENT_BACKEND=]
          [default: exec]
          [possible values: exec, sp1, risc0]

      --proof-coordinators <URL>...
          URLs of all the sequencers' proof coordinator

          [env: PROVER_CLIENT_PROOF_COORDINATOR_URL=]

      --proving-time <PROVING_TIME>
          Time to wait before requesting new data to prove

          [env: PROVER_CLIENT_PROVING_TIME=]
          [default: 5000]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [env: PROVER_CLIENT_LOG_LEVEL=]
          [default: INFO]

      --sp1-server <URL>
          Url to the moongate server to use when using sp1 backend

          [env: ETHREX_SP1_SERVER=]
```

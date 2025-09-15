# Summary

# Introduction

- [Getting started](./getting-started/README.md)
  - [Quickstart Ethereum Node](./getting-started/quickstart-l1.md)
  - [Quickstart L2](./getting-started/quickstart-l2.md)
  - [Hardware requirements]()
  - [Installation](./getting-started/installation/README.md)
    - [Binary distribution](./getting-started/installation/binary_distribution.md)
    - [Package manager](./getting-started/installation/package_manager.md)
    - [Docker image](./getting-started/installation/docker_images.md)
    - [Building from source](./getting-started/installation/building_from_source.md)

# Ethrex for Ethereum node operators

- [Running a node](./l1/running/README.md)
  - [Connecting to a consensus client](./l1/running/consensus_client.md)
  - [Node startup](./l1/running/startup.md)
  - [Configuration](./l1/running/configuration.md)
  - [Monitoring and metrics](./l1/running/monitoring.md)
- [Fundamentals](./l1/fundamentals/README.md)
  - [Metrics]()
  - [Logs]()
  - [Security]()
  - [Databases]()
  - [Networking](./l1/fundamentals/networking.md)
  - [Sync modes](./l1/fundamentals/sync_modes.md)
  - [Pruning]()

# Ethrex for L2 chains

- [Introduction](./l2/introduction.md)
- [Deploy an L2](./l2/deploy.md)
- [Monitoring and metrics](./l2/monitoring.md)
- [Architecture](./l2/architecture/README.md)
  - [Overview](./l2/architecture/overview.md)
  - [Sequencer](./l2/architecture/sequencer.md)
  - [Prover](./l2/architecture/prover.md)
  - [Aligned mode](./l2/architecture/aligned_mode.md)
  - [TDX execution module](./l2/architecture/tdx.md)
- [Interacting with the L2](./l2/interacting/README.md)
  - [Deposit assets](./l2/interacting/deposit.md)
  - [Withdraw assets](./l2/interacting/withdraw.md)
  - [Connect a wallet](./l2/interacting/wallet.md)
  - [Deploy a contract](./l2/interacting/deploy_contracts.md)
  - [Maintain the sequencer]()
- [Fundamentals](./l2/fundamentals/README.md)
  - [State diffs](./l2/fundamentals/state_diffs.md)
  - [Deposits](./l2/fundamentals/deposits.md)
  - [Withdrawals](./l2/fundamentals/withdrawals.md)
  - [Smart contracts](./l2/fundamentals/contracts.md)
    - [OnChainOperator]()
    - [CommonBridge]()
    - [L1MessageSender]()
  - [Based sequencing](./l2/fundamentals/based.md)

# Ethrex for developers

- [Getting started](./developers/README.md)
- [Building](./developers/installing.md)
- [L1](./developers/l1/introduction.md)
  - [Ethrex as a local development node](./developers/l1/dev-mode.md)
  - [Importing blocks from a file](./developers/l1/importing-blocks.md)
  - [Kurtosis localnet](./developers/l1/kurtosis-localnet.md)
  - [Metrics](./developers/l1/metrics.md)
  - [Testing](./developers/l1/testing/README.md)
    - [Ethereum foundation tests](./developers/l1/testing/ef-tests.md)
    - [Hive tests](./developers/l1/testing/hive.md)
    - [Assertoor tests](./developers/l1/testing/assertoor.md)
    - [Rust tests](./developers/l1/testing/rust.md)
    - [Load tests](./developers/l1/testing/load-tests.md)
- [L2](./developers/l2/introduction.md)
  - [Ethrex L2 as local development mode](./developers/l2/dev-mode.md)
- [Debugging solidity with ethrex](./vm/levm/debug.md)
- [Re-execute Ethereum with ethrex](./ethrex_replay/ethrex_replay.md)
  - [Profiling zkvm execution with ethrex replay](./ethrex_replay/profiling.md)
- [CLI reference](./CLI.md)
- [Troubleshooting]()

# Roadmap

- [Roadmap](./roadmap.md)

# Other resources

- [Contributing to the Documentation](./CONTRIBUTING_DOCS.md)

## CLI Commands

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
          [default: ethrex]

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

# Connecting to a consensus client

Ethrex is an execution client built for Ethereum networks after the [merge](https://ethereum.org/en/roadmap/merge/). As a result, ethrex must operate together with a [consensus client](https://ethereum.org/en/developers/docs/nodes-and-clients/#consensus-clients) to fully participate in the network.

### Consensus clients

There are several consensus clients and all of them work with ethrex. When choosing a consensus client we suggest you keep in mind [client diversity](https://ethereum.org/en/developers/docs/nodes-and-clients/client-diversity).

- [Lighthouse](https://lighthouse.sigmaprime.io/)
- [Lodestar](https://lodestar.chainsafe.io/)
- [Nimbus](https://nimbus.team/)
- [Prysm](https://prysm.offchainlabs.com/)
- [Teku](https://consensys.io/teku)
- [Grandine](https://docs.grandine.io/)

## Configuring ethrex

### JWT secret

Consensus clients and execution clients communicate through an authenticated JSON-RPC API. The authentication is done through a [jwt](https://www.jwt.io/) secret. Ethrex automatically generates the jwt secret and saves it to the current working directory by default. You can also use your own previously generated jwt secret by using the `--authrpc.jwtsecret` flag or `JWTSECRET_PATH` environment variable. If the jwt secret at the specified path does not exist ethrex will create it.

### Auth RPC server

By default the server is exposed at `http://localhost:8551` but both the address and the port can be modified using the `--authrpc.addr` and `--authrpc.port` flags respectively.

### Example

```
ethrex --authrpc.jwtsecret path/to/jwt.hex  --authrpc.addr localhost --authrpc.port 8551
```

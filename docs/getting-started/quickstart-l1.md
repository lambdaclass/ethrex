# Quickstart: Run an Ethereum L1 Node

Install ethrex and lighthouse:

```sh
#install lightouse and ethrex
brew install lambdaclass/ethrex
brew install lighthouse
```

Create secrets directory and jwt secret:

```sh
mkdir -p ethereum/secrets/
cd ethereum/
openssl rand -hex 32 | tr -d "\n" | tee ./secrets/jwt.hex
```

On one terminal:

```sh
ethrex --authrpc.jwtsecret ./secrets/jwt.hex --authrpc.addr localhost --authrpc.port 8551 --network hoodi
```

and on another one:

```sh
lighthouse bn --network hoodi --execution-endpoint http://localhost:8551 --execution-jwt ./secrets/jwt.hex --checkpoint-sync-url https://hoodi.checkpoint.sigp.io --http (edited) v
```

- **mainnet**
- **sepolia**
- **holesky**
- **hoodi**

By default, the command below run a node on mainnet. To use a different network, change the `ETHREX_NETWORK` environment variable with one of the networks above.

```sh
curl -LO https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/docker-compose.yaml
ETHREX_NETWORK=mainnet docker compose up
```

This will start an ethrex node along with a Lighthouse consensus client that syncs with the Ethereum network.

---

For more details on installation, flags, and supported networks:

- [Installation](./installation)
- [Advanced options and networks](../l1/running)

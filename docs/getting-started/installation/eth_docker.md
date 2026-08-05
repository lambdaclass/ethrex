# Running ethrex with eth-docker

[eth-docker](https://ethdocker.com) is a community-maintained framework that runs an Ethereum execution+consensus stack from Docker Compose. It includes built-in support for ethrex, so you can stand up a fully containerised node with one configuration file.

This page covers running ethrex as the execution client paired with a consensus client (Lighthouse in the example below). For staking, validator, MEV-Boost and monitoring setups, refer to the [official eth-docker documentation](https://ethdocker.com).

## Prerequisites

- A Linux host that meets the [ethrex hardware requirements](../hardware_requirements.md).
- [Docker Engine](https://docs.docker.com/engine/install/) including the `docker compose` plugin.
- `git` to clone the eth-docker repository.

## Install

1. Clone eth-docker:

   ```sh
   git clone https://github.com/ethstaker/eth-docker.git ~/eth-docker
   cd ~/eth-docker
   ```

2. Seed your local configuration from `default.env`:

   ```sh
   cp default.env .env
   ```

## Configuration

Edit `.env` and set, at minimum, the following variables. The values below run ethrex + Lighthouse on Ethereum mainnet using snap sync with checkpoint sync for the consensus layer:

```ini
CORE_FILES=ethrex.yml:lighthouse-cl-only.yml
NETWORK=mainnet
CHECKPOINT_SYNC_URL=https://mainnet.checkpoint.sigp.io
```

- `CORE_FILES` selects which Compose files participate in the stack. `ethrex.yml` brings up ethrex as the execution client; `lighthouse-cl-only.yml` brings up a Lighthouse beacon node without validators. To use a different consensus client, replace it with the corresponding `<client>-cl-only.yml` file from the eth-docker repository.
- `NETWORK` selects the Ethereum network. ethrex supports `mainnet`, `sepolia`, `holesky`, and `hoodi`.
- `CHECKPOINT_SYNC_URL` is the beacon checkpoint sync endpoint used by the CL. Use an endpoint that matches `NETWORK`; the [eth-clients checkpoint sync list](https://eth-clients.github.io/checkpoint-sync-endpoints/) tracks current options.

`FEE_RECIPIENT` is also required by the consensus client even for non-staking setups, because the beacon node passes `--suggested-fee-recipient` to the engine on every fork-choice update. For a non-validating sync node it does not matter what address is set; the burn address is a common choice:

```ini
FEE_RECIPIENT=0x0000000000000000000000000000000000000000
```

### Choosing the ethrex image

eth-docker can either pull a published ethrex image from GHCR or build one from source. The relevant variables in `.env` are:

```ini
ETHREX_DOCKER_REPO=ghcr.io/lambdaclass/ethrex
ETHREX_DOCKER_TAG=latest
ETHREX_DOCKERFILE=Dockerfile.binary
ETHREX_SRC_REPO=https://github.com/lambdaclass/ethrex
ETHREX_SRC_BUILD_TARGET='$(git describe --tags $(git rev-list --tags --max-count=1))'
```

There are two build modes:

| Mode | `ETHREX_DOCKERFILE` | Image source | When to use |
|------|---------------------|--------------|-------------|
| Binary | `Dockerfile.binary` | Pulls `${ETHREX_DOCKER_REPO}:${ETHREX_DOCKER_TAG}` from a registry. | Fastest path. Works for the published tags only — currently `latest` and `main`. |
| Source | `Dockerfile.source`  | Clones `${ETHREX_SRC_REPO}` and checks out `${ETHREX_SRC_BUILD_TARGET}`, then compiles. | Required when you need to pin to a specific release tag (`vX.Y.Z`, `vX.Y.Z-rc.N`), a feature branch, or a pull request (`pr-1234`). |

**Pinning to a specific release.** The published GHCR repository tracks the rolling `latest` and `main` tags; per-release image tags are not pushed. To run a specific release such as `v13.0.0-rc.2`, switch to source build:

```ini
ETHREX_DOCKERFILE=Dockerfile.source
ETHREX_SRC_BUILD_TARGET=v13.0.0-rc.2
```

The source build clones ethrex inside the container and compiles in release mode. Expect a one-off build time on the order of 10–15 minutes depending on the host; subsequent `docker compose build` invocations reuse the cargo cache.

## Bring up the stack

```sh
docker compose up -d
```

This builds the ethrex image (if needed), pulls the consensus client image, generates a JWT secret on the shared volume, and starts both containers.

Check that the containers are healthy:

```sh
docker compose ps
```

Tail the execution client logs:

```sh
docker compose logs -f execution
```

Tail the consensus client logs:

```sh
docker compose logs -f consensus
```

Once both clients have established peers and the CL has finished checkpoint sync, the EL will begin snap-syncing state. Confirm progress with a JSON-RPC call against the host:

```sh
curl -s http://localhost:8545 \
    -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":1}'
```

A response of `{"jsonrpc":"2.0","id":1,"result":false}` means the node is fully synced.

## Stop, restart, and resync

Stop the stack:

```sh
docker compose down
```

Restart after configuration changes (re-reads `.env`):

```sh
docker compose up -d
```

Resync from scratch by removing the named volumes:

```sh
docker compose down
docker volume rm eth-docker_ethrex-el-data eth-docker_lhconsensus-data
docker compose up -d
```

Volume names are prefixed with the Compose project name, which by default is the directory containing `.env` (`eth-docker`). The CL volume name depends on the consensus client (e.g. `eth-docker_lhconsensus-data` for Lighthouse, `eth-docker_prysmconsensus-data` for Prysm); list them with `docker volume ls | grep eth-docker` if unsure.

## Architecture

eth-docker runs ethrex and the consensus client as two separate containers on a shared Compose network. They communicate over the Engine API using a JWT secret stored on the `jwtsecret` named volume.

```
                ┌───────────────────────┐
   JSON-RPC ──▶ │  execution (ethrex)   │  ⇄  ports 30303/tcp+udp (P2P)
   :8545 / :8546 └────────────┬──────────┘
                              │ Engine API :8551 (JWT-authenticated)
                ┌─────────────▼──────────┐
                │  consensus (lighthouse)│  ⇄  port  9000/tcp+udp (P2P)
                └────────────────────────┘
```

The `ethrex.yml` Compose file invokes the ethrex binary with the following defaults:

- `--datadir /var/lib/ethrex` (persisted on the `ethrex-el-data` named volume)
- `--authrpc.jwtsecret /var/lib/ee-secret/jwtsecret` (shared with the CL)
- `--http.addr 0.0.0.0 --http.port 8545`
- `--ws.enabled --ws.addr 0.0.0.0 --ws.port 8546`
- `--metrics --metrics.addr 0.0.0.0 --metrics.port 6060`
- `--p2p.port 30303 --discovery.port 30303`

eth-docker selects the sync mode via the `EL_NODE_TYPE` variable (default `pre-merge-expiry`):

| `EL_NODE_TYPE` | `mainnet` / `sepolia` | `hoodi` / `holesky` |
|----------------|-----------------------|---------------------|
| `pre-merge-expiry` | `--syncmode snap` | `--syncmode snap` |
| `full` | `--syncmode snap` (full sync is not yet supported by ethrex on these networks; eth-docker falls back to snap) | `--syncmode full` |
| `archive` | not supported | not supported |

Additional ethrex CLI flags can be appended through the `EL_EXTRAS` variable in `.env`, for example to expose extra RPC namespaces:

```ini
EL_EXTRAS="--http.api eth,net,web3,debug"
```

> The `admin_*`, `debug_*`, and `txpool_*` namespaces are unauthenticated; only enable them when the RPC port is restricted by firewall or a reverse proxy.

## References

- eth-docker documentation: <https://ethdocker.com>
- eth-docker source: <https://github.com/ethstaker/eth-docker>
- eth-docker ethrex Compose file: <https://github.com/ethstaker/eth-docker/blob/main/ethrex.yml>
- ethrex CLI reference: [CLI](../../CLI.md)
- ethrex sync modes: [Sync modes](../../l1/fundamentals/sync_modes.md)
- Checkpoint sync endpoints: <https://eth-clients.github.io/checkpoint-sync-endpoints/>

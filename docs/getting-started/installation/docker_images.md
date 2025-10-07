# Installing ethrex (docker)

Run Ethrex easily using Docker containers. This guide covers pulling and running official images.

## Prerequisites

- [Docker](https://www.docker.com/get-started/) installed and running

## Pulling the Docker Image

**Latest stable release:**

```sh
docker pull ghcr.io/lambdaclass/ethrex:latest
```

**Latest development build:**

```sh
docker pull ghcr.io/lambdaclass/ethrex:main
```

**Specific version:**

```sh
docker pull ghcr.io/lambdaclass/ethrex:<version-tag>
```

Find available tags in the <a href="https://github.com/lambdaclass/ethrex/tags" target="_blank">GitHub repo</a>.

---

## Running the Docker Image

### Check the Image

Verify the image is working:

```sh
docker run --rm ghcr.io/lambdaclass/ethrex --version
```

### Start an ethrex Node

Run the following command to start a node in the background:

```sh
docker run \
    --rm \
    -d \
    -v ethrex:/root/.local/share/ethrex \
    -p 8545:8545 \
    -p 8551:8551 \
    -p 30303:30303 \
    -p 30303:30303/udp \
    -p 9090:9090 \
    --name ethrex \
    ghcr.io/lambdaclass/ethrex \
    --authrpc.addr 0.0.0.0
```

**What this does:**

- Starts a container named `ethrex`
- Publishes ports:
  - `8545`: JSON-RPC server (TCP)
  - `8551`: Auth JSON-RPC server (TCP)
  - `30303`: P2P networking (TCP/UDP)
  - `9090`: Metrics (TCP)
- Mounts the Docker volume `ethrex` to persist blockchain data

**Tip:** You can add more Ethrex CLI arguments at the end of the command as needed.

---

## Managing the Container

**View logs:**

```sh
docker logs -f ethrex
```

**Stop the node:**

```sh
docker stop ethrex
```

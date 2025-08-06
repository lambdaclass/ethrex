# Docker images

## Prerequisites

- [Docker](https://www.docker.com/get-started/)

## Pull the docker image

To pull the latest stable docker image run

```
docker pull ghcr.io/lambdaclass/ethrex:latest
```

To pull the latest development docker image run

```
docker pull ghcr.io/lambdaclass/ethrex:unstable
```

To pull the image for an specific version

```
docker pull ghcr.io/lambdaclass/ethrex:<version-tag>
```

Existing tags are available in the [GitHub repo](https://github.com/lambdaclass/ethrex/tags)

## Run the docker image

> [!NOTE]
> If you previously pulled `latest` or `unstable` tags Docker will run the already cached version
> Make sure the image is up to date by [pulling the docker image](#pull-the-docker-image)

### Verify the image is working

```
docker run --rm ghcr.io/lambdaclass/ethrex --version
```

### Start the node, publish default ports and persist data

```
docker run \
    --rm -d \
    -v ethrex:/root/.local/share/ethrex \
    --name ethrex \
    ghcr.io/lambdaclass/ethrex \
    --http.addr 0.0.0.0 --authrpc.addr 0.0.0.0
```

This command will start a container called `ethrex` that by default exposes the following ports

- `8545`: TCP port for the JSON-RPC server
- `8551`: TCP port for the auth JSON-RPC server
- `30303`: TCP/UDP port for p2p networking
- `9090`: TCP port metrics port
- `1729`: TCP port for the Layer 2 JSON-RPC server
- `3900`: TCP port for the Layer 2 proof coordinator server

The command also mounts the docker volume ethrex to persist data.

If you want to follow the logs run
```
docker logs -f ethrex 
```

To stop the container run

```
docker stop ethrex
```

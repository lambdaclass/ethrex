# Docker images

## Prerequisites

- [Docker](https://www.docker.com/get-started/)

## Pull the docker image

To pull the latest stable docker image, run:

```
docker pull ghcr.io/lambdaclass/ethrex:latest
```

To pull the latest development docker image, run:

```
docker pull ghcr.io/lambdaclass/ethrex:unstable
```

To pull the image for a specific version, run:

```
docker pull ghcr.io/lambdaclass/ethrex:<version-tag>
```

Existing tags are available in the [GitHub repo](https://github.com/lambdaclass/ethrex/tags)

## Run the docker image

### Verify the image is working

```
docker run --rm ghcr.io/lambdaclass/ethrex --version
```

### Start the node

```
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
    --http.addr 0.0.0.0 \
    --authrpc.addr 0.0.0.0
```

This command will start a container called `ethrex` and publish the following ports

- `8545`: TCP port for the JSON-RPC server
- `8551`: TCP port for the auth JSON-RPC server
- `30303`: TCP/UDP port for p2p networking
- `9090`: TCP port metrics port

The command also mounts the docker volume `ethrex` to persist data.

If you want to follow the logs run
```
docker logs -f ethrex 
```

To stop the container run

```
docker stop ethrex
```

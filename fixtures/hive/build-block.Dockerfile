# Builds and runs the EELS (execution-specs) build-block simulator, which tests
# block-building correctness via the `testing_buildBlockV1` HTTP RPC method
# (execution-specs PR #2679). Mirrors the upstream eels/consume-engine
# simulator Dockerfile but swaps the entrypoint to `uv run build-block`.
#
# This file is copied into the local hive clone by `make run-hive-build-block`
# at simulators/ethereum/eels/build-block/Dockerfile. Once an
# `ethereum/eels/build-block` simulator lands in hive upstream, this can be
# dropped.
FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim

## Default fixtures/git-ref
ARG fixtures=stable@latest
ENV FIXTURES=${fixtures}
ARG branch=""

## Clone and install EELS
RUN apt-get update && apt-get install -y git

# Clone repo and use the default branch if none specified
RUN git clone --depth 1 https://github.com/ethereum/execution-specs.git && \
    cd execution-specs && \
    if [ -n "$branch" ]; then \
        git fetch --depth 1 origin "$branch" && \
        git checkout FETCH_HEAD; \
    fi

WORKDIR /execution-specs/packages/testing

# Cache the fixtures so they are not re-downloaded on every container start.
# Rebuild the image (or pass `--docker.nocache`) to pick up newer fixtures.
RUN uv sync
RUN uv run consume cache --input "$FIXTURES"

## Run the block-building simulator against the local fixtures.
ENTRYPOINT uv run build-block -v --input "$FIXTURES"

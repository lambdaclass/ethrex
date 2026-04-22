FROM rust:1.91 AS chef

RUN apt-get update && apt-get install -y \
    build-essential \
    libclang-dev \
    libc6 \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef

WORKDIR /ethrex


# --- Planner Stage ---
# Copy all source code to calculate the dependency recipe.
# This layer is fast and will be invalidated on any source change.
FROM chef AS planner

COPY benches ./benches
COPY crates ./crates
COPY metrics ./metrics
COPY cmd ./cmd
COPY test ./test
COPY tooling ./tooling
COPY Cargo.* .
COPY .cargo/ ./.cargo

RUN cargo chef prepare --recipe-path recipe.json


# --- Builder Stage ---
# Build the dependencies. This is the most time-consuming step.
# This layer will be cached and only re-run if the recipe.json from the
# previous stage has changed, which only happens when dependencies change.
FROM chef AS builder

# Build configuration
# PROFILE: Cargo profile to use (release, release-with-debug-assertions, etc.)
# BUILD_FLAGS: Additional cargo flags (features, etc.)
ARG PROFILE="release"
ARG BUILD_FLAGS=""

COPY --from=planner /ethrex/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json $BUILD_FLAGS

RUN  if [ "$(uname -m)" = aarch64 ]; \
    then \
    SOLC_URL=https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux-arm;\
    PROTOC_URL=https://github.com/protocolbuffers/protobuf/releases/download/v29.3/protoc-29.3-linux-aarch_64.zip;\
    else \
    SOLC_URL=https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux; \
    PROTOC_URL=https://github.com/protocolbuffers/protobuf/releases/download/v29.3/protoc-29.3-linux-x86_64.zip;\
    fi \
    && curl -L -o /usr/bin/solc $SOLC_URL \
    && chmod +x /usr/bin/solc \
    && curl -L -o /tmp/protoc.zip $PROTOC_URL \
    && unzip -o /tmp/protoc.zip -d /usr/local bin/protoc \
    && rm /tmp/protoc.zip

COPY benches ./benches
COPY crates ./crates
COPY cmd ./cmd
COPY metrics ./metrics
COPY tooling ./tooling
COPY fixtures/genesis ./fixtures/genesis
COPY .git ./.git
COPY Cargo.* ./
COPY fixtures ./fixtures
COPY .cargo/ ./.cargo

ENV COMPILE_CONTRACTS=true

RUN cargo build --profile $PROFILE $BUILD_FLAGS

RUN mkdir -p /ethrex/bin && \
    cp /ethrex/target/${PROFILE}/ethrex /ethrex/bin/ethrex

# --- Final Image ---
# Copy the ethrex binary into a minimalist image to reduce bloat size.
# This image must have glibc and libssl
FROM ubuntu:24.04
WORKDIR /usr/local/bin

RUN apt-get update && apt-get install -y --no-install-recommends libssl3

COPY cmd/ethrex/networks ./cmd/ethrex/networks
COPY --from=builder /ethrex/bin/ethrex .

# Common ports:
# -  8545: RPC
# -  8551: EngineAPI
# - 30303: Discovery
# -  9090: Metrics
# -  1729: L2 RPC
# -  3900: L2 Proof Coordinator
EXPOSE 8545
EXPOSE 8551
EXPOSE 30303/tcp
EXPOSE 30303/udp
EXPOSE 9090
EXPOSE 1729
EXPOSE 3900

ENTRYPOINT [ "./ethrex" ]

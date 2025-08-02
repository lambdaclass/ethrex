FROM rust:1.87 AS chef

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

COPY crates ./crates
COPY tooling ./tooling
COPY metrics ./metrics
COPY cmd ./cmd
COPY Cargo.* .

RUN cargo chef prepare --recipe-path recipe.json


# --- Builder Stage ---
# Build the dependencies. This is the most time-consuming step.
# This layer will be cached and only re-run if the recipe.json from the
# previous stage has changed, which only happens when dependencies change.
FROM chef AS builder

COPY --from=planner /ethrex/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY crates ./crates
COPY cmd ./cmd
COPY metrics ./metrics
COPY Cargo.* ./
COPY fixtures ./fixtures

# Optional build flags
ARG BUILD_FLAGS=""
RUN cargo build --release $BUILD_FLAGS

# --- Final Image ---
# Copy the ethrex binary into a minimalist image to reduce bloat size.
# This image must have glibc and libssl
FROM gcr.io/distroless/cc-debian12
WORKDIR /usr/local/bin

COPY cmd/ethrex/networks ./cmd/ethrex/networks
COPY --from=builder /ethrex/target/release/ethrex .

EXPOSE 8545
EXPOSE 8551
EXPOSE 1729
EXPOSE 3900

ENTRYPOINT [ "./ethrex" ]

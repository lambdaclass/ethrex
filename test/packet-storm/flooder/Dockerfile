# Reuse the ethrex builder stage which already has all deps installed+compiled.
# This avoids needing apt-get (which may fail with Docker DNS issues).
FROM rust:1.90 AS chef

RUN cargo install cargo-chef

WORKDIR /ethrex

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

FROM chef AS builder

COPY --from=planner /ethrex/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY benches ./benches
COPY crates ./crates
COPY cmd ./cmd
COPY metrics ./metrics
COPY tooling ./tooling
COPY fixtures ./fixtures
COPY .git ./.git
COPY Cargo.* ./
COPY test/ ./test/
COPY .cargo/ ./.cargo

# Add flooder to workspace for the build
RUN sed -i '/^members = \[/a\  "test/packet-storm/flooder",' Cargo.toml

RUN cargo build --release -p packet-storm-flooder

# Minimal final image — static-enough binary, no apt needed
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /ethrex/target/release/packet-storm-flooder /flooder
ENTRYPOINT ["/flooder"]

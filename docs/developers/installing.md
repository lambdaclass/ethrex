# Setting up a development environment for ethrex

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [Git](https://git-scm.com/downloads)
- [Docker](https://www.docker.com/get-started/)

## Cloning the repo

The full code of ethrex is available at [GitHub](https://github.com/lambdaclass/ethrex) and can be cloned using git

```
git clone https://github.com/lambdaclass/ethrex && cd ethrex
```

## Building the ethrex binary

Ethrex can be built using cargo

To build the client run
```
cargo build --release --bin ethrex
```

the following feature can be enable with `--features <features>`

|Feature|Description|
|-------|-----------|
|**default**|Enables "rocksdb", "c-kzg", "rollup_storage_sql", "dev", "metrics" features|
|debug|Enables [debug mode](../vm/levm/debug.md) for LEVM|
|**dev**|Makes the [--dev](./l1/dev-mode.md) flag available|
|**metrics**|Enables metrics gathering for use with a monitoring stack|
|**c-kzg**|Enables the c-kzg crate instead of kzg-rs|
|**rocksdb**|Enables rocksdb as the database for the ethereum state|
|**rollup_storage_sql**|Enables sql as the database for the L2 batch data|
|sp1|Enables the sp1 backend for the L2 prover|
|risc0|Enables the risc0 backend for the L2 prover|
|gpu|Enables CUDA support for the zk backends risc0 and sp1|

**Bolded** are features enabled by default

Additionally the environment variable `COMPILE_CONTRACTS` can be set to `true` to enable embedding the solidity contracts used by the rollup, into the binary to enable the [L2 dev mode](../developers/l2/dev-mode.md).

## Building the docker image

The Dockerfile is located at the root of the repository and can be built by running

```
docker build -t ethrex .
```

The `BUILD_FLAGS` argument can be used to pass flags to cargo, for example

```
docker build -t ethrex --build-arg BUILD_FLAGS="--features <features>" .
```

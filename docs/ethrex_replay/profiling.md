# Profiling zkvms with ethrex-replay

## Getting started

Before reading this document please take a look the general documentation for [ethrex-replay](./ethrex_replay.md)

### Dependencies

#### For SP1

The easiest way is to use cargo but other options are listed in the [samply repo](https://github.com/mstange/samply)
```
cargo install --locked samply
```

#### For risc0

Install go by following the instructions from [go install page](https://go.dev/doc/install)

### Generate a profile

#### For SP1

**Profile a L1 block**: 

Required: `RPC_URL`.
Optionally: `BLOCK_NUMBER`, `NETWORK`.
```sh
make profile-sp1
```

**Profile a L2 batch**:

Required: `RPC_URL`, `BATCH_NUMBER`, `NETWORK`.

```sh
make profile-batch-sp1
```

**Open samply profile**

```sh
samply load output.json
```
Then visit http://localhost:8000/ on your browser

#### For risc0

**Profile a L1 block**: 

Required: `RPC_URL`.
Optionally: `BLOCK_NUMBER`, `NETWORK`.
```sh
make profile-risc0
```

**Profile a L2 batch**:

Required: `RPC_URL`, `BATCH_NUMBER`, `NETWORK`.

```sh
make profile-batch-risc0
```

**Open pprof profile**

```sh
go tool pprof -http=127.0.0.1:8000 profile.pb
```
Then visit http://localhost:8000/ on your browser

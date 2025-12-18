# Building ethrex from source

Build ethrex yourself for maximum flexibility and experimental features.

## Prerequisites

- [Rust toolchain](https://www.rust-lang.org/tools/install) (use `rustup` for easiest setup)
- [libclang](https://clang.llvm.org/docs/index.html) (for RocksDB)
- [Git](https://git-scm.com/downloads)
- [solc (v0.8.31)](https://docs.soliditylang.org/en/v0.8.31/installing-solidity.html) (for L2 development)

### L2 contracts

If you want to install ethrex for L2 development, you may set the `COMPILE_CONTRACTS` env var, so the binary have the necessary contract code.

```sh
export COMPILE_CONTRACTS=true
```

## Install via `cargo install`

The fastest way to install ethrex from source:

```sh
cargo install --locked ethrex --git https://github.com/lambdaclass/ethrex.git
```

**Optional features:**

- Add `--features sp1,risc0` to enable SP1 and/or RISC0 provers
- Add `--features gpu` for CUDA GPU support

**Install a specific version:**

```sh
cargo install --locked ethrex --git https://github.com/lambdaclass/ethrex.git --tag <version-tag>
```

Find available tags in the <a href="https://github.com/lambdaclass/ethrex/tags" target="_blank">GitHub repo</a>.

**Verify installation:**

```sh
ethrex --version
```

---

## Build manually with `cargo build`

Clone the repository (replace `<version-tag>` with the desired version):

```sh
git clone --branch <version-tag> --depth 1 https://github.com/lambdaclass/ethrex.git
cd ethrex
```

Build the binary:

```sh
cargo build --bin ethrex --release
```

**Optional features:**

- Add `--features sp1,risc0` to enable SP1 and/or RISC0 provers
- Add `--features gpu` for CUDA GPU support

The built binary will be in `target/release/ethrex`.

**Verify the build:**

```sh
./target/release/ethrex --version
```

**(Optional) Move the binary to your `$PATH`:**

```sh
sudo mv ./target/release/ethrex /usr/local/bin/
```

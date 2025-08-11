# Building from source

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [Git](https://git-scm.com/downloads)

## Installing using `cargo install`

To install the client simply run 

```sh
cargo install --locked ethrex --git https://github.com/lambdaclass/ethrex.git
```

> [!TIP]
> You can add `sp1` and `risc0` features to the installation script to build with support for SP1
> and/or RISC0 provers. `gpu` feature is also available for CUDA support.

To install a specifc version you can add the `--tag <tag>` flag.
Existing tags are available in the [GitHub repo](https://github.com/lambdaclass/ethrex/tags)


After that, you can verify the program is working by running:

```sh
ethrex --version
```

## Building the binary with `cargo build`

You can download the source code of a release from the [GitHub releases page](https://github.com/lambdaclass/ethrex/releases), or by cloning the repository at that version:

```sh
git clone --branch <LATEST_VERSION_HERE> --depth 1 https://github.com/lambdaclass/ethrex.git
```

After that, you can run the following command inside the cloned repo to build the client:

```sh
cargo build --bin ethrex --release
```

> [!TIP]
> You can add `sp1` and `risc0` features to the installation script to build with support for SP1
> and/or RISC0 provers. `gpu` feature is also available for CUDA support.

You can find the built binary inside `target/release` directory.
After that, you can verify the program is working by running:

```sh
./target/release/ethrex --version
```

> [!TIP]
> For convenience, you can move the `ethrex` binary to a directory in your `$PATH`, so you can run it from anywhere.

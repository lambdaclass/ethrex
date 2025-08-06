# Building from source

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [Git](https://git-scm.com/downloads) (Only if building the binary with `cargo build`)

## Installing using cargo install

To install the client simply run 

```sh
cargo install --locked ethrex --git https://github.com/lambdaclass/ethrex.git
```

To install a specifc version you can add the `--tag <tag>` flag.
Existing tags are available in the [GitHub repo](https://github.com/lambdaclass/ethrex/tags)


After that, you can verify the program is working by running:

```sh
ethrex --version
```

## Building the binary with cargo build

You can download the source code of a release from the [GitHub releases page](https://github.com/lambdaclass/ethrex/releases), or by cloning the repository at that version:

```sh
git clone --branch <LATEST_VERSION_HERE> --depth 1 https://github.com/lambdaclass/ethrex.git
```

After that, you can run the following command inside the cloned repo to build the client:

```sh
cargo build --bin ethrex --release
```

You can find the built binary inside `target/release` directory.
After that, you can verify the program is working by running:

```sh
./target/release/ethrex --version
```

> [!TIP]
> For convenience, you can move the `ethrex` binary to a directory in your `$PATH`, so you can run it from anywhere.

# Installing ethrex

Before [running the ethrex client](./running.md), you need to install it.
We have multiple ways to install, depending on your needs.

Pre-built binaries for Linux and macOS are available for each release.
See [Installing from pre-built binaries](#installing-from-pre-built-binaries) for installation instructions.

You can also [build the client from source](#building-from-source).

## Installing from pre-built binaries

To install ethrex from pre-built binaries, first download the binaries for your platform from the [release page](https://github.com/lambdaclass/ethrex/releases).
You can also download it from the command line using `curl` or `wget`:

```sh
# For Linux x86_64
curl -L https://github.com/lambdaclass/ethrex/releases/download/v0.0.1-rc.1/ethrex-linux_x86_64 -o ethrex
wget https://github.com/lambdaclass/ethrex/releases/download/v0.0.1-rc.1/ethrex-linux_x86_64 -O ethrex

# For Linux ARM
curl -L https://github.com/lambdaclass/ethrex/releases/download/v0.0.1-rc.1/ethrex-linux_aarch64 -o ethrex
wget https://github.com/lambdaclass/ethrex/releases/download/v0.0.1-rc.1/ethrex-linux_aarch64 -O ethrex

# For MacOS
curl -L https://github.com/lambdaclass/ethrex/releases/download/v0.0.1-rc.1/ethrex-macos_aarch64 -o ethrex
wget https://github.com/lambdaclass/ethrex/releases/download/v0.0.1-rc.1/ethrex-macos_aarch64 -O ethrex
```

And set the execution bit:

```sh
chmod +x ethrex
```

After that, you can verify the program is working by running:

```sh
./ethrex --version
```

This should output something like:

```text
ethrex ethrex/v0.1.0-HEAD-d3aa87a/aarch64-apple-darwin/rustc-v1.87.0
```

> [!TIP]
> For convenience, you can move the `ethrex` binary to a directory in your `PATH`, so you can run it from anywhere.

After installing the client, see ["Running the client"](./running.md) for instructions on how to use it to run L1 and/or L2 networks.

## Building from source

To install the client, [first install Rust](https://www.rust-lang.org/tools/install) and then run:

```sh
cargo install --locked ethrex \
    --git https://github.com/lambdaclass/ethrex.git \
    --features dev
```

This installs the `ethrex` binary.
For more information on how it is built and installed, see [the cargo-install documentation](https://doc.rust-lang.org/cargo/commands/cargo-install.html).

After installing the client, see ["Running the client"](./running.md) for instructions on how to use it to run L1 and/or L2 networks.

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

> [!TIP]
> For convenience, you can move the `ethrex` binary to a directory in your `PATH`, so you can run it from anywhere.

## Building from source

To install the client, [first install Rust](https://www.rust-lang.org/tools/install) and then run:

```sh
cargo install --locked ethrex \
    --git https://github.com/lambdaclass/ethrex.git \
    --features dev
```

This installs the `ethrex` binary.

# Installing ethrex

We have multiple ways to install the client, depending on your needs.

Pre-built binaries for Linux and macOS are available for each release.
See [Installing from pre-built binaries](#installing-from-pre-built-binaries) for installation instructions.

You can also [build the client from source](#building-from-source).

## Installing from pre-built binaries

To install ethrex from pre-built binaries, first download the binaries for your platform from the [release page](https://github.com/lambdaclass/ethrex/releases).

After that, extract the downloaded archive:

```sh
tar -xvf ethrex*.tar.gz
```

And set the execution bit:

```sh
chmod +x ethrex
```

After that, you can run the client as follows:

```sh
./ethrex
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

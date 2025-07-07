# Installing ethrex

We have multiple ways to install the client, depending on your needs.
[Pre-compiled binaries are available for each release](./installing_prebuilt.md), but you can also [build the client from source](./installing_source.md).

## Installing from pre-built binaries

This part is a work-in-progress and will be updated soon.
For now, the only way to install ethrex is by [building it from source](#building-from-source).

## Building from source

To install the client, [first install Rust](https://www.rust-lang.org/tools/install) and run:

```sh
cargo install --locked ethrex \
    --git https://github.com/lambdaclass/ethrex.git \
    --features dev
```

This installs the `ethrex` binary.

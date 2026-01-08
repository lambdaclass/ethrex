# Step by Step

1. clone zisk:

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
```

1. checkout to the custom branch:

```bash
cd zisk && git checkout feature/bn128
```

1. Build ZisK:

```bash
cargo build --release --features gpu
```

1. Follow step from 3 to 7 in the [installation guide](https://github.com/0xPolygonHermez/zisk/blob/feature/bn128/book/getting_started/installation.md#option-2-building-from-source) from the building from source


1. Do the rom setup:

```bash
cargo-zisk rom-setup -e <PATH_TO_ELF> -k ~/.zisk/provingKey
```

1. Check the setup:

```bash
cargo-zisk check-setup -k ~/.zisk/provingKey -a
```


1. Export libs:

```bash
export LD_LIBRARY_PATH=/home/admin/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/lib
```

1. Prove an input:

```bash
cargo-zisk prove -e <PATH_TO_ELF> -i <PATH_TO_INPUT> -a -u -f -k ~/.zisk/provingKey -w <PATH_TO_ZISK_REPO>/target/release/libzisk_witness.so
```


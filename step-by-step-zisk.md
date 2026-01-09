# Step by Step

1. clone zisk:

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
```

2. checkout to the custom branch:

```bash
cd zisk && git checkout feature/bn128
```

3. Build ZisK:

```bash
cargo build --release --features gpu
```

4. Follow step from 3 to 7 in the [installation guide](https://github.com/0xPolygonHermez/zisk/blob/feature/bn128/book/getting_started/installation.md#option-2-building-from-source) from the building from source

5. Download the public keys:

```bash
curl -LO https://storage.googleapis.com/zisk-setup/zisk-0.15.0-plonk.tar.gz 
```

6. Extract the keys:


```bash
tar -xvzf zisk-0.15.0-plonk.tar.gz -C zisk-pkey
```

7. copy the keys to the path:

```bash
cd zisk-pkey
cp -r provingKey ~/.zisk
cp -r provingKeySnark ~/.zisk
```

8. Do the rom setup:

```bash
cargo-zisk rom-setup -e <PATH_TO_ELF> -k ~/.zisk/provingKey
```

9. Check the setup:

```bash
cargo-zisk check-setup -k ~/.zisk/provingKey -a
```


10. Export libs:

```bash
export LD_LIBRARY_PATH=/home/admin/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/lib
```

11. Prove an input:

```bash
cargo-zisk prove -e <PATH_TO_ELF> -i <PATH_TO_INPUT> -a -u -f -k ~/.zisk/provingKey -w <PATH_TO_ZISK_REPO>/target/release/libzisk_witness.so
```


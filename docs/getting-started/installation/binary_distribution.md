# Install binary distribution

## Download the binary

Download the latest ethrex release for your OS from the [packaged binaries](https://github.com/lambdaclass/ethrex/releases)

```sh
# For Linux x86_64
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64 -o ethrex
wget https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64 -O ethrex

# For Linux ARM
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_aarch64 -o ethrex
wget https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_aarch64 -O ethrex

# For MacOS
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-macos_aarch64 -o ethrex
wget https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-macos_aarch64 -O ethrex
```


Set the executable flag by running

```
chmod +x ethrex
```

After that, you can verify the program is working by running:

```sh
./ethrex --version
```

> [!TIP]
> For convenience, you can move the `ethrex` binary to a directory in your `$PATH`, so you can run it from anywhere.

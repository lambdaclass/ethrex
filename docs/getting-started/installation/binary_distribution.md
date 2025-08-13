# Install binary distribution

## Download the binary

Download the latest ethrex release for your OS from the [packaged binaries](https://github.com/lambdaclass/ethrex/releases)

#### For Linux x86_64:
```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64 -o ethrex
```

#### For Linux ARM:
```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_aarch64 -o ethrex
```

#### For MacOS (Apple Silicon):
```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-macos_aarch64 -o ethrex
```

## Give execution permissions to the binary

```
chmod +x ethrex
```

Finally, you can verify the program is working by running:

```sh
./ethrex --version
```

> [!TIP]
> For convenience, you can move the `ethrex` binary to a directory in your `$PATH`, so you can run it from anywhere.

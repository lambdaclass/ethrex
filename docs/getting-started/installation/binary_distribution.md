# Install ethrex (binary distribution)

This guide explains how to quickly install the latest pre-built ethrex binary for your operating system.

## Prerequisites

- [curl](https://curl.se/download.html) (for downloading the binary)

## Download the latest release

Download the latest ethrex release for your OS from the <a href="https://github.com/lambdaclass/ethrex/releases/latest" target="_blank">GitHub Releases page</a>.

### Linux x86_64

```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64 -o ethrex
```

#### Linux x86_64 with GPU support (for L2 prover)

If you want to run an L2 prover with GPU acceleration, download the GPU-enabled binary:

```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64-gpu -o ethrex
```

### Linux ARM (aarch64)

```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_aarch64 -o ethrex
```

#### Linux ARM (aarch64) with GPU support (for L2 prover)

If you want to run an L2 prover with GPU acceleration, download the GPU-enabled binary:

```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_aarch64-gpu -o ethrex
```

### macOS (Apple Silicon, aarch64)

```sh
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-macos_aarch64 -o ethrex
```

## Set execution permissions

Make the binary executable:

```sh
chmod +x ethrex
```

## (Optional) Move to a directory in your `$PATH`

To run `ethrex` from anywhere, move it to a directory in your `$PATH` (e.g., `/usr/local/bin`):

```sh
sudo mv ethrex /usr/local/bin/
```

## Verify the installation

Check that Ethrex is installed and working:

```sh
ethrex --version
```

---

**Next steps:**

- [L1 Quickstart](../quickstart-l1.md)
- [L2 Quickstart](../quickstart-l2.md)

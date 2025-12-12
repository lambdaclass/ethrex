# Ethrex Pre-Release Checklist

This checklist helps ensure a smooth pre-release process for ethrex. Before proceeding, replace placeholders like `X.Y.Z-rc.N` with the actual version number (e.g., `1.2.3-rc.1`). Mark each test as completed by checking the box (e.g., `- [x]`) or adding a ✅ or notes in the table cells. If a test doesn't apply, note "N/A" with a brief reason.

Refer to the ethrex documentation for detailed setup instructions. All tests should be run on a clean environment to avoid interference from previous builds.

## 1. Test the Binaries

### Prerequisites

Before testing, download the [pre-release binaries from the release artifacts](https://github.com/lambdaclass/ethrex/releases).

### Running the Tests

- **Snapsync Hoodi**: Follow the [snap sync guide for Hoodi](https://docs.ethrex.xyz/l1/running/index.html) to initialize and sync a test network.
- **L2 Integration Tests**: Follow the [L2 integration tests guide](https://docs.ethrex.xyz/developers/l2/integration-tests.html).

| Binary                        | Snapsync Hoodi | L2 Integration Tests |
|-------------------------------|----------------|----------------------|
| ethrex-macos-aarch64         |                | N/A (L2-bin-specific) |
| ethrex-linux-aarch64         |                | N/A (L2-bin-specific)   |
| ethrex-linux-x86_64          |                | N/A (L2-bin-specific)   |
| ethrex-l2-macos-aarch64      |                |                      |
| ethrex-l2-linux-aarch64      |                |                      |
| ethrex-l2-linux-x86_64       |                |                      |
| ethrex-l2-linux-aarch64-gpu  |                |                      |
| ethrex-l2-linux-x86_64-gpu   |                |                      |

## 2. Test the Docker Images

### Prerequisites

- Follow the [Docker installation guide](https://docs.ethrex.xyz/getting-started/installation/docker_images.html?highlight=docker#installing-ethrex-docker).

### Running the Tests

- **Snapsync Hoodi**: Run the container with snap sync flags and confirm it syncs a test network successfully.
- **L2 Integration Tests**: Same as with the binary.

#### 2.1 Using `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N`

These are standard images without L2 support.

| Target Arch          | Snapsync Hoodi |
|--------------------|----------------|
| macos-aarch64     |                |
| linux-aarch64     |                |
| linux-x86_64      |                |

#### 2.2 Using `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N-l2`

These images include L2 features.

| Target Arch          | Snapsync Hoodi | L2 Integration Tests |
|--------------------|----------------|----------------------|
| macos-aarch64     |                |                      |
| linux-aarch64     |                |                      |
| linux-x86_64      |                |                      |

## Next Steps After Testing

- If all tests pass: Proceed to full release validation.
- If failures occur: Report the errors.

Happy testing!

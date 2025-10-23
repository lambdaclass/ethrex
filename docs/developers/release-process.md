# How to Release an ethrex version

Releases are prepared from dedicated release branches and tagged using versioning.

## 1st - Create release branch

Branch name must follow the format `release/vX.Y.Z`.

Examples:

- `release/v1.2.0`
- `release/v3.0.0`
- `release/v3.2.0`

## 2nd - Bump version

In the release branch, update the `[workspace.package]` version to `X.Y.Z` in the root `Cargo.toml`, and push the change to the branch.

An example can be found here:

https://github.com/lambdaclass/ethrex/pull/4881/files#diff-2e9d962a08321605940b5a657135052fbcef87b5e360662bb527c96d9a615542

There are currently three `Cargo.lock` files that will be affected. Make sure you check them:

- root `Cargo.lock`
- `sp1/Cargo.lock`
- `risc0/Cargo.lock`

## 3rd - Create & Push Tag

Create a tag with a format `vX.Y.Z-rc.W` where `X.Y.Z` is the semantic version and `W` is a release candidate version. Other names for subversions are also accepted. Example of valid tags:

- `v0.1.3-rc.1`
- `v0.0.2-alpha`

```bash
git tag <release_version>
git push origin <release_version>
```

After pushing the tag, a CI job will compile the binaries for different architectures and create a pre-release with the version specified in the tag name. Along with the binaries, a tar file is uploaded with the contracts and the verification keys. The following binaries are built:

| name | L2 stack | Provers | CUDA support |
| --- | --- | --- | --- |
| ethrex-linux-x86-64 | ❌ | - | - |
| ethrex-linux-aarch64 | ❌ | - | - |
| ethrex-linux-macos-aarch64 | ❌ | - | - |
| ethrex-l2-linux-x86-64 | ✅ | SP1 - RISC0 - Exec | ❌ |
| ethrex-l2-linux-x86-64-gpu | ✅ | SP1 - RISC0 - Exec | ✅ |
| ethrex-l2-linux-aarch64 | ✅ | SP1 - Exec | ❌ |
| ethrex-l2-linux-aarch64-gpu | ✅ | SP1 - Exec | ✅ |
| ethrex-l2-macos-aarch64 | ✅ | Exec | ❌ |

Also, two docker images are built and pushed to the Github Container registry:
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W`
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2`

A changelog will be generated based on commit names (using conventional commits) from the last stable tag.

## 4th - Test & Publish Release

When you are sure all the binaries and docker images work as expected, you can proceed to publish the release. To do so, edit the last pre-release with the following changes:
- Change the name to `ethrex: vX.Y.Z`
- Change the tag to a new one `vX.Y.Z`. **IMPORTANT**: Make sure to select the `release/vX.Y.Z` branch when changing the tag.
- Set the release as the latest release (you will need to uncheck the pre-release first).

Once done, the CI will publish new tags for the already compiled docker images:
- `ghcr.io/lambdaclass/ethrex:X.Y.Z`, `ghcr.io/lambdaclass/ethrex:latest`
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-l2`, `ghcr.io/lambdaclass/ethrex:l2`

## 5th - Update Homebrew

Disclaimer: We should automate this

1. Commit a change in https://github.com/lambdaclass/homebrew-tap/ bumping the ethrex version (like [this one](https://github.com/lambdaclass/homebrew-tap/commit/d78a2772ad9c5412e7f84c6210bd85c970fcd0e6)).
    - The first SHA is the hash of the `.tar.gz` from the release. You can get it by downloading the `Source code (tar.gz)` from the ethrex release and running

        ```bash
        shasum -a 256 ethrex-v3.0.0.tar.gz
        ```

    - For the second one:
        - First download the `ethrex-l2-macos-aarch64` binary from the ethrex release
        - Give exec permissions to binary

            ```bash
            chmod +x ethrex-l2-macos-aarch64
            ```

        - Create a dir `ethrex/3.0.0/bin` (replace the version as needed)
        - Move (and rename) the binary to `ethrex/3.0.0/bin/ethrex` (the last `ethrex` is the binary)
        - Remove quarantine flags (in this case, `ethrex` is the root dir mentioned before):

            ```bash
            xattr -dr com.apple.metadata:kMDItemWhereFroms ethrex
            xattr -dr com.apple.quarantine ethrex
            ```

        - Tar the dir with the following name (again, `ethrex` is the root dir):

            ```bash
            tar -czf ethrex-3.0.0.arm64_sonoma.bottle.tar.gz ethrex
            ```

        - Get the checksum:

            ```bash
            shasum -a 256 ethrex-3.0.0.arm64_sonoma.bottle.tar.gz
            ```

        - Use this as the second hash (the one in the `bottle` section)
2. Push the commit
3. Create a new release with tag `v3.0.0`. **IMPORTANT**: attach the `ethrex-3.0.0.arm64_sonoma.bottle.tar.gz` to the release

## 6th - Merge the release branch via PR

Once the release is verified, **merge the branch via PR**.

## Dealing with hotfixes

If hotfixes are needed before the final release, commit them to `release/vX.Y.Z`, push, and create a new pre-release tag. The final tag `vX.Y.Z` should always point to the exact commit you will merge via PR.

# How to Release an ethrex version

Releases are prepared from dedicated release branches and tagged using versioning.

## 1st - Create release branch

Branch name must follow the format `release/vX.Y.Z`.

Examples:

- `release/v1.2.0`
- `release/v3.0.0`
- `release/v3.2.0`

## 2nd - Bump version

The version must be updated to `X.Y.Z` in the release branch. There are multiple `Cargo.toml` and `Cargo.lock` files that need to be updated.

First, we need to update the version of the workspace package. You can find it in the `Cargo.toml` file in the root directory, under the `[workspace.package]` section. This is also the version the library crates are published under on crates.io when the release is finalized, so it must be a clean semver version (the `-rc.W` suffix lives only on the git tag).

Then, we need to update five more `Cargo.toml` files that are not part of the workspace but fulfill the role of packages in the monorepo. These are located in the following paths:

- `crates/guest-program/bin/sp1/Cargo.toml`
- `crates/guest-program/bin/risc0/Cargo.toml`
- `crates/guest-program/bin/zisk/Cargo.toml`
- `crates/guest-program/bin/openvm/Cargo.toml`
- `crates/l2/tee/quote-gen/Cargo.toml`

We also need to bump the **internal crate dependency version pins**. ethrex's library crates declare their workspace-internal dependencies with an explicit `version = "X.Y.Z"` (required for publishing to crates.io). These live in the root `Cargo.toml` under `[workspace.dependencies]`, plus a few crates that pin a sibling directly (`crates/vm/Cargo.toml`, `crates/blockchain/Cargo.toml`, `crates/l2/sdk/Cargo.toml`). Bump every one of them to the new version, then confirm none were missed (substitute the previous release version):

```bash
grep -rn '"<previous version>"' --include=Cargo.toml .   # must return nothing
```

> [!WARNING]
> The version bump must cover **both** `[workspace.package].version` and these dependency pins. If you bump only the workspace version, the release-branch PR still merges cleanly, but the merged result declares every crate at the new version while requiring the **previous** version of its siblings — which breaks the entire workspace build (every `cargo` job fails to resolve).

After updating the version in the `Cargo.toml` files, we need to update the `Cargo.lock` files to reflect the new versions. Run `make update-cargo-lock` from the root directory to update all the `Cargo.lock` files in the repository. You should see changes in at most the following paths:

- In the root directory
- `crates/guest-program/bin/sp1/Cargo.lock`
- `crates/guest-program/bin/risc0/Cargo.lock`
- `crates/guest-program/bin/zisk/Cargo.lock`
- `crates/guest-program/bin/openvm/Cargo.lock`
- `crates/l2/tee/quote-gen/Cargo.lock`
- `crates/vm/levm/bench/revm_comparison/Cargo.lock`
- `tooling/Cargo.lock`

Then, go to the `CLI.md` file located in `docs/` and update the version of the `--builder.extra-data` flag default value to match the new version (for both ethrex and ethrex l2 sections).

Finally, stage and commit the changes to the release branch.

An example of a PR that bumps the version can be found [here](https://github.com/lambdaclass/ethrex/pull/4881/files#diff-2e9d962a08321605940b5a657135052fbcef87b5e360662bb527c96d9a615542).

## 3rd - Create & Push Tag

Create a tag with a format `vX.Y.Z-rc.W` where `X.Y.Z` is the semantic version and `W` is a release candidate version. Other names for subversions are also accepted. Example of valid tags:

- `v0.1.3-rc.1`
- `v0.0.2-alpha`

```bash
git tag <release_version>
git push origin <release_version>
```

After pushing the tag, a CI job will compile the binaries for different architectures and create a pre-release with the version specified in the tag name. Along with the binaries, a tar file is uploaded with the contracts and the verification keys. The following binaries are built:

| name | L1 | L2 stack | Provers | CUDA support |
| --- | --- | --- | --- | --- |
| ethrex-linux-x86_64 | ✅ | ❌ | - | - |
| ethrex-linux-aarch64 | ✅ | ❌ | - | - |
| ethrex-macos-aarch64 | ✅ | ❌ | - | - |
| ethrex-l2-linux-x86_64 | ✅ | ✅ | SP1 - RISC0 - Exec | ❌ |
| ethrex-l2-linux-x86_64-gpu | ✅ | ✅ | SP1 - RISC0 - Exec | ✅ |
| ethrex-l2-linux-aarch64 | ✅ | ✅ | SP1 - Exec | ❌ |
| ethrex-l2-linux-aarch64-gpu| ✅ | ✅ | SP1 - Exec | ✅ |
| ethrex-l2-macos-aarch64 | ✅ | ✅ | Exec | ❌ |

Also, two docker images are built and pushed to the Github Container registry:
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W`
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2`

A changelog will be generated based on commit names (using conventional commits) from the last stable tag.

## 4th - Test & Publish Release

### Testing checklist

Before publishing the release, run through the following checks using the pre-release binaries:

- [ ] Upgrade `ethrex-ethdocker-mainnet`
- [ ] Upgrade `ethrex-mainnet-1`
- [ ] Upgrade `ethrex-minimum-mainnet`
- [ ] Launch multisync on `ethrex-multisync-main`
- [ ] Upgrade a local L2 created with the previous version and run the integration tests
- [ ] Run the L2 integration tests with a SP1 prover on the GPU server (`l2-gpu`)

The commands for each target follow. The host roster changes between releases — fill in the ones you run and leave the placeholders for the rest. Replace `vX.Y.Z-rc.W` / `release/vX.Y.Z` with the version under test.

#### `ethrex-ethdocker-mainnet`

```bash
ssh admin@ethrex-ethdocker-mainnet
cd eth-docker/
# In .env, set the release tag on this line:
#   ETHREX_SRC_BUILD_TARGET=vX.Y.Z-rc.W
nano .env
./ethd update
./ethd up
```

#### `ethrex-mainnet-1`

<!-- TODO: document the upgrade procedure for this host -->

#### `ethrex-minimum-mainnet`

<!-- TODO: document the upgrade procedure for this host -->

#### `ethrex-multisync-main`

```bash
ssh admin@ethrex-multisync-main
cd ethrex/tooling/sync/
tmux new-session -d -s sync "make multisync-loop-auto MULTISYNC_BRANCH=release/vX.Y.Z 2>&1"
```

#### Local L2 upgrade + integration tests

See [Upgrade test](l2/upgrade-test.md) for the full procedure.

#### L2 integration tests with a SP1 prover (`l2-gpu`)

This validates the release's SP1 **GPU** proving path end to end: bring up an L1 + L2 testnet from the **release binaries** with the SP1 GPU prover, then run the integration test against it. Run it on a host with an NVIDIA GPU (`l2-gpu`).

> [!NOTE]
> The standard [integration tests](l2/integration-tests.md) run the prover in `exec` mode (no real proofs). This variant swaps in the SP1 GPU backend so the release's proving path is actually exercised. GPU/driver setup (CUDA, `nvidia-container-toolkit`, the `docker` group) is covered in [Prover § GPU mode](l2/prover.md#gpu-mode) and [Run an ethrex L2 SP1 prover](../l2/deployment/prover/sp1.md); the SP1 wrap step runs in a `moongate` Docker container, so `docker run --gpus all` must work.

Host prerequisites: `solc` **0.8.31 exactly** (the FeeToken pragmas are pinned), `foundry` (`cast`/`forge`) on `PATH`, and the Rust toolchain from `rust-toolchain.toml`.

1. **Download the release artifacts** (note: asset names use `x86_64`, with an underscore):

    ```bash
    export TAG=vX.Y.Z-rc.W
    mkdir -p ~/ethrex_$TAG && cd ~/ethrex_$TAG
    BASE=https://github.com/lambdaclass/ethrex/releases/download/$TAG
    curl -sSL -o ethrex                  "$BASE/ethrex-linux-x86_64"        &
    curl -sSL -o ethrex-l2               "$BASE/ethrex-l2-linux-x86_64-gpu" &
    curl -sSL -o ethrex-contracts.tar.gz "$BASE/ethrex-contracts.tar.gz"    &
    curl -sSL -o ethrex-guests.tar.gz    "$BASE/ethrex-guests.tar.gz"       &
    wait
    chmod +x ethrex ethrex-l2
    mkdir -p contracts && tar xzf ethrex-contracts.tar.gz -C contracts
    ```

    Genesis files and rich-account keys aren't published as assets — copy them from a checkout at the same tag:

    ```bash
    git show $TAG:fixtures/genesis/l1.json          > l1.json
    git show $TAG:fixtures/genesis/l2.json          > l2.json
    git show $TAG:fixtures/keys/private_keys_l1.txt > private_keys_l1.txt
    ```

2. **Start L1** (dev mode auto-mines):

    ```bash
    nohup ./ethrex --network l1.json \
      --http.addr 0.0.0.0 --http.port 8545 \
      --authrpc.addr 0.0.0.0 --authrpc.port 8551 \
      --dev --datadir dev_ethrex_l1 > l1.log 2>&1 &
    ```

3. **Deploy the L1 contracts with SP1 enabled** (`solc`/`forge` must be on `PATH`):

    ```bash
    COMPILE_CONTRACTS=true ./ethrex-l2 l2 deploy \
      --eth-rpc-url http://localhost:8545 \
      --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
      --sp1 true \
      --on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
      --bridge-owner            0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
      --bridge-owner-pk         0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
      --deposit-rich \
      --private-keys-file-path private_keys_l1.txt \
      --genesis-l1-path l1.json --genesis-l2-path l2.json \
      --sp1-vk-path contracts/ethrex-riscv32im-succinct-zkvm-vk-bn254 \
      --inclusion-max-wait 86400 > deploy.log 2>&1
    grep -E "(Timelock|OnChainProposer|CommonBridge|SP1Verifier) deployed" deploy.log
    ```

    > [!WARNING]
    > `--sp1-vk-path` must point at the verification key **inside `contracts/`** (`contracts/ethrex-riscv32im-succinct-zkvm-vk-bn254`), not the similarly named `guests/sp1/...` file from `ethrex-guests.tar.gz`. The wrong file registers a VK the binary's embedded ELF never produces, and `lastVerifiedBatch` stays stuck at `0` while `lastCommittedBatch` climbs. `--inclusion-max-wait 86400` avoids a privileged-transaction deadlock when the test bursts ~300 transactions. The deploy fails at the very end writing `.env` to a CI-baked path — that's harmless; take the addresses from `deploy.log`.

4. **Start the L2 sequencer** (substitute the addresses from `deploy.log`):

    ```bash
    nohup ./ethrex-l2 l2 --no-monitor \
      --watcher.block-delay 0 --network l2.json \
      --http.addr 0.0.0.0 --http.port 1729 --metrics --metrics.port 3702 \
      --datadir dev_ethrex_l2 \
      --l1.bridge-address            <BRIDGE_FROM_DEPLOY> \
      --l1.on-chain-proposer-address <PROPOSER_FROM_DEPLOY> \
      --l1.timelock-address          <TIMELOCK_FROM_DEPLOY> \
      --eth.rpc-url http://localhost:8545 \
      --osaka-activation-time 1761677592 \
      --block-producer.coinbase-address           0x0007a881CD95B1484fca47615B64803dad620C8d \
      --block-producer.base-fee-vault-address     0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
      --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
      --block-producer.operator-fee-per-gas 1000000000 \
      --committer.l1-private-key         0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
      --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
      --proof-coordinator.addr 127.0.0.1 \
      --log.color never > l2.log 2>&1 &
    ```

    `--no-monitor` is mandatory when running headless — otherwise the binary starts the monitor TUI and emits no logs.

5. **Start the SP1 GPU prover** (first run pulls the `moongate` container and loads ~15 GB into VRAM):

    ```bash
    nohup ./ethrex-l2 l2 prover --backend sp1 \
      --proof-coordinators tcp://127.0.0.1:3900 --log.level info > prover.log 2>&1 &
    ```

6. **Sanity check** (after ~3 min both should be non-zero and climbing):

    ```bash
    PROP=<PROPOSER_FROM_DEPLOY>
    cast call $PROP 'lastCommittedBatch()(uint256)' --rpc-url http://localhost:8545
    cast call $PROP 'lastVerifiedBatch()(uint256)'  --rpc-url http://localhost:8545
    ```

    If `lastVerifiedBatch` stays at `0` while `lastCommittedBatch` climbs, you picked the wrong `--sp1-vk-path` (see the warning above).

7. **Run the integration test** against the running testnet, from a source checkout at the same tag.

    The test harness loads contract addresses from `cmd/.env` in the checkout. The deployer normally writes this file, but the release binary targets a baked-in CI path and fails to (see step 3), so recreate it from the deploy output — it needs the proposer/bridge **and** the verifier/DAO entries (a missing verifier address such as `ETHREX_DEPLOYER_TDX_CONTRACT_VERIFIER` makes the test fail). Backends you didn't deploy keep their sentinel/zero defaults:

    ```bash
    git clone --depth 1 --branch $TAG https://github.com/lambdaclass/ethrex.git ~/ethrex_${TAG}_src
    cd ~/ethrex_${TAG}_src

    cat > cmd/.env <<'EOF'
    ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=<PROPOSER_FROM_DEPLOY>
    ETHREX_WATCHER_BRIDGE_ADDRESS=<BRIDGE_FROM_DEPLOY>
    ETHREX_DEPLOYER_SP1_CONTRACT_VERIFIER=<SP1_VERIFIER_FROM_DEPLOY>
    ETHREX_DEPLOYER_PICO_CONTRACT_VERIFIER=0x00000000000000000000000000000000000000aa
    ETHREX_DEPLOYER_RISC0_CONTRACT_VERIFIER=0x00000000000000000000000000000000000000aa
    ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS=0x00000000000000000000000000000000000000aa
    ETHREX_DEPLOYER_TDX_CONTRACT_VERIFIER=<TDX_VERIFIER_FROM_DEPLOY>
    ENCLAVE_ID_DAO=0x0000000000000000000000000000000000000000
    FMSPC_TCB_DAO=0x0000000000000000000000000000000000000000
    PCK_DAO=0x0000000000000000000000000000000000000000
    EOF

    INTEGRATION_TEST_L1_RPC=http://localhost:8545 \
    INTEGRATION_TEST_L2_RPC=http://localhost:1729 \
    ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=~/ethrex_$TAG/private_keys_l1.txt \
    INTEGRATION_TEST_PRIVATE_KEYS_FILE_PATH=~/ethrex_$TAG/private_keys_l1.txt \
    INTEGRATION_TEST_BRIDGE_OWNER_PRIVATE_KEY=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
    cargo test -p ethrex-test l2:: --release --features l2 -- --nocapture --test-threads=1
    ```

    Pass criterion: `test result: ok. 1 passed; 0 failed`, plus the final `Total L2 ETH == Bridge locked ETH on L1` reconciliation line. Expect a multi-hour run on the GPU backend (proving dominates the wall-clock). See [integration tests § "taking too long"](l2/integration-tests.md#i-think-my-tests-are-taking-too-long-how-can-i-debug-this) if it appears to stall.

### Publish

Once the pre-release is created and you want to publish the release, go to the [release page](https://github.com/lambdaclass/ethrex/releases) and follow the next steps:

1. Click on the edit button of the last pre-release created

    ![edit button](../img/publish_release_step_1.png)

2. Manually create the tag `vX.Y.Z`. **Set the tag's _Target_ to the release branch (`release/vX.Y.Z`), not the default `main`** — otherwise the tag lands on `main`'s HEAD, which is not the version-bumped commit.

    ![edit tag](../img/publish_release_step_2.png)

    Before finalizing, verify the tag points at the exact commit you tested (the release candidate):

    ```bash
    git ls-remote origin refs/tags/vX.Y.Z refs/tags/vX.Y.Z-rc.W   # the two SHAs must match
    ```

    > [!WARNING]
    > The crates.io publish workflow reads `publish.yml` from `main` but checks out the **tag's commit**. If the tag points at the wrong commit (e.g. an unbumped `main`), the workflow republishes the *previous* version's crates — each is "already published", so the run goes **green while publishing nothing new**. The final `vX.Y.Z` tag must point at the same commit as the tested `vX.Y.Z-rc.W` (the release branch HEAD you will merge via PR).

3. Update the release title

    ![edit title](../img/publish_release_step_3.png)

4. Customize the release notes.

    The auto-generated changelog lists every commit, but it doesn't tell operators what actually matters in this release. Above the auto-generated changelog, add a hand-written summary using [GitHub alerts](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#alerts). Pick boxes by what the operator needs to decide, in this order:

    - `> [!IMPORTANT]` — **only** when the release carries critical security or correctness fixes. One line stating that upgrading is strongly recommended for all operators.
    - `> [!WARNING]` — **only** when the upgrade can't be cleanly undone or carries a breaking change the operator must account for: a database schema migration you can't roll back from, a required resync, removed or renamed CLI flags / config options, changed defaults, breaking RPC/API changes, or new minimum requirements (disk, dependency, consensus-client version). State what changes and what the operator must do.
    - `> [!NOTE]` — **always**. A **What's new** list of the highlights (new features, important fixes), plus a line saying whether a resync is needed (if not already covered above).

    Keep the space after `>` (`> [!NOTE]`, not `>[!NOTE]`) and leave a blank line between boxes so each renders separately. Drop the `[!IMPORTANT]` / `[!WARNING]` boxes when they don't apply — a routine release needs only the `[!NOTE]`.

    ```markdown
    > [!IMPORTANT]
    > This release contains critical fixes. Upgrading is strongly recommended for all operators.

    > [!WARNING]
    > This release changes the database schema; once you upgrade you can't roll back to a previous version. The migration runs automatically.

    > [!NOTE]
    > **What's new**
    > - <highlight>
    > - <highlight>
    >
    > No resync is needed.
    ```

5. Set the release as the latest release (you will need to uncheck the pre-release first). And finally, click on `Update release`

    ![set latest release](../img/publish_release_step_4.png)

Once done, the CI will publish new tags for the already compiled docker images:

- `ghcr.io/lambdaclass/ethrex:X.Y.Z`, `ghcr.io/lambdaclass/ethrex:latest`
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-l2`, `ghcr.io/lambdaclass/ethrex:l2`

Promoting the pre-release to a full release also publishes ethrex's library crates to crates.io — see the next section.

### Publishing to crates.io

Promoting the pre-release to a full release (the `released` event from the step above) triggers the `publish.yml` workflow, which runs `cargo publish` for ethrex's publishable library crates in dependency order, at the workspace version `X.Y.Z`.

> [!NOTE]
> Only the **final** release publishes to crates.io. Pre-release (`vX.Y.Z-rc.W`) tags do **not**: the `released` event does not fire for pre-releases, and crates.io versions are immutable, so a release candidate must never claim the version before it has been tested.

The crates are published at the `[workspace.package].version` bumped in step 2; the `-rc.W` suffix lives only on the git tag and never reaches crates.io.

This requires a one-time organizational setup before the first release that publishes:

- A `CRATES_IO_TOKEN` repository secret with publish rights for the crates.
- A `crates-release-prod` GitHub environment (the workflow runs inside it).
- crates.io ownership of the crate names (the first publish under the token claims them).

The workflow is idempotent: a crate version already on crates.io is skipped, so re-running after a partial failure is safe. To validate without publishing, run it manually from the Actions tab (the `workflow_dispatch` trigger) with the dry-run input checked — it lists each crate's package contents instead of publishing.

## 5th - Update Homebrew

Disclaimer: We should automate this

Set the released version once, then the commands below are copy/paste:

```bash
export V=X.Y.Z   # replace with the released version, no `v` prefix (e.g. 3.0.0)
```

1. Commit a change in https://github.com/lambdaclass/homebrew-tap/ bumping the ethrex version (like [this one](https://github.com/lambdaclass/homebrew-tap/commit/d78a2772ad9c5412e7f84c6210bd85c970fcd0e6)). It needs two SHA-256 hashes:

    - **Source tarball hash** (the `url` field) — download the GitHub source archive and hash it:

        ```bash
        curl -L -o "ethrex-v$V.tar.gz" "https://github.com/lambdaclass/ethrex/archive/refs/tags/v$V.tar.gz"
        shasum -a 256 "ethrex-v$V.tar.gz"
        ```

    - **Bottle hash** (the `bottle` section) — build the macOS bottle from the release binary. The `ethrex` directory is the bottle root:

        ```bash
        # download the L2 macOS binary from the ethrex release
        gh release download "v$V" -R lambdaclass/ethrex -p ethrex-l2-macos-aarch64
        chmod +x ethrex-l2-macos-aarch64

        # lay it out as ethrex/<version>/bin/ethrex (last `ethrex` is the binary)
        mkdir -p "ethrex/$V/bin"
        mv ethrex-l2-macos-aarch64 "ethrex/$V/bin/ethrex"

        # strip quarantine flags (root dir is ./ethrex)
        xattr -dr com.apple.metadata:kMDItemWhereFroms ethrex
        xattr -dr com.apple.quarantine ethrex

        # tar and hash the bottle (root dir is ./ethrex)
        tar -czf "ethrex-$V.arm64_sonoma.bottle.tar.gz" ethrex
        shasum -a 256 "ethrex-$V.arm64_sonoma.bottle.tar.gz"
        ```

2. Push the commit.
3. Create a new release with tag `v$V` in homebrew-tap. **IMPORTANT**: attach the `ethrex-$V.arm64_sonoma.bottle.tar.gz` to the release.

## 6th - Merge the release branch via PR

Once the release is verified, **merge the branch via PR**.

## Dealing with hotfixes

If hotfixes are needed before the final release, commit them to `release/vX.Y.Z`, push, and create a new pre-release tag. The final tag `vX.Y.Z` should always point to the exact commit you will merge via PR.

## Troubleshooting

### Failure on "latest release" workflow

If the CI fails when setting a release as latest (step 5), Docker tags `latest` and `l2` may not be updated. To manually push those changes, follow these steps:

- Create a new Github Personal Access Token (PAT) from the [settings](https://github.com/settings/tokens/new).
- Check `write:packages` permission (this will auto-check `repo` permissions too), give a name and a short expiration time.
- Save the token securely.
- Click on `Configure SSO` button and authorize LambdaClass organization.
- Log in to Github Container Registry: `docker login ghcr.io`. Put your Github's username and use the token as your password.
- Pull RC images:

```bash
docker pull --platform linux/amd64 ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W
docker pull --platform linux/amd64 ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2
```

- Retag them:

```bash
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W ghcr.io/lambdaclass/ethrex:X.Y.Z
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2 ghcr.io/lambdaclass/ethrex:X.Y.Z-l2
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W ghcr.io/lambdaclass/ethrex:latest
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2 ghcr.io/lambdaclass/ethrex:l2
```

- Push them:

```bash
docker push ghcr.io/lambdaclass/ethrex:X.Y.Z
docker push ghcr.io/lambdaclass/ethrex:X.Y.Z-l2
docker push ghcr.io/lambdaclass/ethrex:latest
docker push ghcr.io/lambdaclass/ethrex:l2
```

- Delete the PAT for security ([here](https://github.com/settings/tokens))

### Failure on the crates.io publish workflow

If `publish.yml` fails partway through, fix the cause and re-run the workflow. Crates already published at the release version are skipped (the run tolerates an "already exists" error), so it resumes from the first crate that has not been published yet. Because crates are published in dependency order, a metadata or ordering error in one crate blocks the crates that depend on it, while the ones published before it stay published (crates.io versions cannot be unpublished or overwritten — a fix requires a new version).

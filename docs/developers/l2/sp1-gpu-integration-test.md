# L2 integration tests with a SP1 GPU prover

This validates a release's SP1 **GPU** proving path end to end: bring up an L1 + L2 testnet from the **release binaries** with the SP1 GPU prover, then run the integration test against it. Run it on a host with an NVIDIA GPU (`l2-gpu`). It is one of the checks in the [release testing checklist](../release-process.md#testing-checklist).

> [!NOTE]
> The standard [integration tests](integration-tests.md) run the prover in `exec` mode (no real proofs). This variant swaps in the SP1 GPU backend so the release's proving path is actually exercised. GPU/driver setup (CUDA, `nvidia-container-toolkit`, the `docker` group) is covered in [Prover § GPU mode](prover.md#gpu-mode) and [Run an ethrex L2 SP1 prover](../../l2/deployment/prover/sp1.md); the SP1 wrap step runs in a `moongate` Docker container, so `docker run --gpus all` must work.

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

    Genesis files and rich-account keys aren't published as assets — get them from a source checkout at the same tag (the test in step 7 reuses this checkout):

    ```bash
    git clone --depth 1 --branch $TAG https://github.com/lambdaclass/ethrex.git ~/ethrex_${TAG}_src
    cp ~/ethrex_${TAG}_src/fixtures/genesis/l1.json          l1.json
    cp ~/ethrex_${TAG}_src/fixtures/genesis/l2.json          l2.json
    cp ~/ethrex_${TAG}_src/fixtures/keys/private_keys_l1.txt private_keys_l1.txt
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

7. **Run the integration test** against the running testnet, from the source checkout cloned in step 1.

    The test reads the deployed contract addresses from `cmd/.env` in the checkout — only `ETHREX_WATCHER_BRIDGE_ADDRESS` and `ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS` (the verifier/DAO `ETHREX_DEPLOYER_*` values are deploy-time *inputs*, not read by the test). The deployer normally writes this file, but the release binary targets a baked-in CI path and fails to (see step 3), so create it from the addresses in `deploy.log`:

    ```bash
    cd ~/ethrex_${TAG}_src

    cat > cmd/.env <<'EOF'
    ETHREX_WATCHER_BRIDGE_ADDRESS=<BRIDGE_FROM_DEPLOY>
    ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=<PROPOSER_FROM_DEPLOY>
    EOF

    INTEGRATION_TEST_L1_RPC=http://localhost:8545 \
    INTEGRATION_TEST_L2_RPC=http://localhost:1729 \
    ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=~/ethrex_$TAG/private_keys_l1.txt \
    INTEGRATION_TEST_PRIVATE_KEYS_FILE_PATH=~/ethrex_$TAG/private_keys_l1.txt \
    INTEGRATION_TEST_BRIDGE_OWNER_PRIVATE_KEY=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
    cargo test -p ethrex-test l2:: --release --features l2 -- --nocapture --test-threads=1
    ```

    Pass criterion: `test result: ok. 1 passed; 0 failed`, plus the final `Total L2 ETH == Bridge locked ETH on L1` reconciliation line. Expect a multi-hour run on the GPU backend (proving dominates the wall-clock). See [integration tests § "taking too long"](integration-tests.md#i-think-my-tests-are-taking-too-long-how-can-i-debug-this) if it appears to stall.

8. **Clean up** — the L1, sequencer, and prover were started with `nohup` and keep running; stop them (and the prover's GPU container) when you're done:

    ```bash
    pkill -f 'ethrex-l2 l2 prover'        # prover first
    pkill -f 'ethrex-l2 l2 --no-monitor'  # sequencer
    pkill -f 'ethrex --network l1.json'   # L1
    docker rm -f "$(docker ps -q --filter ancestor=public.ecr.aws/succinct-labs/moongate)" 2>/dev/null || true
    ```

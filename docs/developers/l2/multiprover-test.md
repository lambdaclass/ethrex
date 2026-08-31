# Multiprover test (SP1 GPU + TDX)

A per-release acceptance test for the configuration a real rollup runs: an `OnChainProposer` that requires **both** an SP1 proof and a TDX proof before it will verify a batch.

The other L2 checks each exercise one prover. This one is the only place where a batch has to satisfy two independent provers, running on two machines, before `lastVerifiedBatch` moves.

## Attestation runs in dev mode, and the TDX guest is a plain VM

Deploy with `ETHREX_TDX_DEV_MODE=true`, and boot the prover VM as an **ordinary QEMU guest** — no `confidential-guest-support=tdx`, no `tdx-guest` object. This is CI's configuration, and the two settings go together.

In dev mode `TDXVerifier.register()` sets

```solidity
authorizedSignature = _getAddress(quote, 0);   // first 20 bytes of the quote
```

and returns, skipping `verifyAndAttestOnChain`. A plain guest emits a dev quote whose leading bytes *are* the prover's address, so that assignment is correct. Boot the same image as a real TDX guest and `quote-gen` produces a genuine quote beginning `0400 0200 81…` — a quote header, not an address — so the contract registers nonsense as the authorized signer, and every `verifyBatch` reverts with `InvalidTdxProof()` (`0x62013a95`) while registration itself appears to succeed.

Running the real TDX path instead would need `ETHREX_TDX_DEV_MODE=false`, which requires the quote's collateral (PCK certs, TCB info) in the PCCS contracts. The sequencer loads it by shelling out to `automata-dcap-qpl-tool`, which resolves addresses from a registry keyed by chain id and knows only public networks: on the dev L1 (chain id 9) it exits `Unsupported chain_id: 9`. That failure is silent, because `prepare_quote_prerequisites` discards the tool's exit status, so the first symptom is a later revert.

**So this test covers the two-prover verification path with a real SP1 GPU proof, and does not exercise TDX hardware or on-chain DCAP verification** — the same coverage CI has. Because the guest is plain, the test needs no TDX-capable CPU; it needs a GPU. Extending it to real attestation is future work and needs a chain the DCAP registry recognises.

## Where to run it

Two hosts, with the SP1 prover reaching the proof coordinator over the tailnet:

| Host | Runs |
| --- | --- |
| `ethrex-tdx-baremetal` | L1, contract deploy, sequencer + proof coordinator, TDX prover VM |
| `l2-gpu` | SP1 prover |

`ethrex-tdx-baremetal` is the designated host for the TDX side, and is where extending this test to real attestation would happen — though as described above, the dev-mode configuration this test uses does not depend on its TDX hardware.

Both are shared machines running other people's work, so keep the footprint small: check that ports 8545, 1729 and 3900 are free before starting, and tear the stack down afterwards. On `l2-gpu`, this test and the [SP1 GPU integration test](sp1-gpu-integration-test.md) contend for the same GPU, ports and datadirs — run them one after the other, never concurrently.

## Environment pins

Four things must be right, and each fails in a way that does not point at its cause:

- **solc must be exactly 0.8.31.** `TDXVerifier.sol` declares `pragma solidity =0.8.31`, so 0.8.30 fails with "Source file requires different compiler version". Install it locally rather than changing the system compiler on a shared host.
- **Deploy onto a fresh chain.** The deploy is not idempotent: CREATE2 puts the dependency contracts at fixed addresses, so re-running against a chain that already has them reverts in `deploy-p256` with no reason string. Wipe the L1 datadir between attempts.
- **Set `--watcher.watch-interval 1000`.** At the 12000 ms default, on a `--dev` L1 that mines instantly, the deposits the sequencer has processed drift out of step with the bridge's pending queue, and every commit reverts with `InvalidPrivilegedTransactionLogs()` (`0x9e6e5638`). CI sets 1000 ms for the same reason.
- **Attach both provers before the first batch is committed, and reset the L1 and L2 datadirs together.** The coordinator hands out work starting at `1 + lastVerifiedBatch`, and prover inputs for old batches are eventually pruned. A prover that joins after batches have accumulated — or a sequencer restarted against an L1 that already has committed batches — asks forever for a batch whose input no longer exists, reports `No blocks to prove`, and nothing advances: verification needs both proofs, so `lastVerifiedBatch` never moves and the request never changes. Wiping only one of the two datadirs reproduces this reliably.

The TDX prover image (`image.raw`) is built with nix from `crates/l2/tee/quote-gen`. If the host has no nix, build it on another host and copy it over — it is ~700 MB and depends only on the release, not the host.

## Procedure

Set the release under test once:

```bash
export TAG=vX.Y.Z-rc.W
```

### 1. Prepare `ethrex-tdx-baremetal`

```bash
# release binaries and contracts (the vk the deployer registers)
mkdir -p ~/multiprover/$TAG && cd ~/multiprover/$TAG
for a in ethrex-linux-x86_64 ethrex-l2-linux-x86_64; do
  curl -fsSL -o $a "https://github.com/lambdaclass/ethrex/releases/download/$TAG/$a" && chmod +x $a
done
mkdir -p contracts && curl -fsSL \
  "https://github.com/lambdaclass/ethrex/releases/download/$TAG/ethrex-contracts.tar.gz" | tar xz -C contracts

# source at the tag, for genesis, keys and the contract build
git clone --depth 1 --branch $TAG https://github.com/lambdaclass/ethrex.git src

# pinned solc, local to the test
curl -fsSL -o bin/solc https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux
chmod +x bin/solc && export PATH=$PWD/bin:$PATH
```

The TDX deploy also needs `automata-dcap-qpl-tool` built from `crates/l2/tee/contracts`. If the host has no Rust toolchain, build it elsewhere and copy the binary into `src/crates/l2/tee/contracts/automata-dcap-qpl/automata-dcap-qpl-tool/target/release/`.

### 2. Start L1 and deploy with both provers required

```bash
./ethrex-linux-x86_64 --network src/fixtures/genesis/l1.json \
  --http.addr 0.0.0.0 --http.port 8545 --authrpc.port 8551 --dev --datadir dev_l1 &

cd src/crates/l2          # the deployer resolves tee/contracts relative to cwd
COMPILE_CONTRACTS=true ETHREX_TDX_DEV_MODE=true \
  ~/multiprover/$TAG/ethrex-l2-linux-x86_64 l2 deploy \
    --eth-rpc-url http://localhost:8545 \
    --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
    --sp1 true --sp1-vk-path ~/multiprover/$TAG/contracts/ethrex-riscv32im-succinct-zkvm-vk-bn254 \
    --tdx true \
    --on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
    --bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
    --bridge-owner-pk 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
    --deposit-rich --private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
    --genesis-l1-path ../../fixtures/genesis/l1.json \
    --genesis-l2-path ../../fixtures/genesis/l2.json \
    --inclusion-max-wait 86400 \
    --env-file-path ~/multiprover/$TAG/.env
```

`--sp1 true --tdx true` together are what make this a multiprover test: the `OnChainProposer` then requires a proof of each kind per batch.

### 3. Start the sequencer

The proof coordinator must bind `0.0.0.0` so the SP1 prover on the other host can reach it, and the qpl tool path must be passed even in dev mode.

```bash
set -a; . ~/multiprover/$TAG/.env; set +a
~/multiprover/$TAG/ethrex-l2-linux-x86_64 l2 --no-monitor \
  --watcher.block-delay 0 --watcher.watch-interval 1000 \
  --network ../../fixtures/genesis/l2.json \
  --http.addr 0.0.0.0 --http.port 1729 --datadir ~/multiprover/$TAG/dev_l2 \
  --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  --l1.on-chain-proposer-address "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" \
  --l1.timelock-address "$ETHREX_TIMELOCK_ADDRESS" \
  --eth.rpc-url http://localhost:8545 --committer.commit-time 15000 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.tdx-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.addr 0.0.0.0 \
  --proof-coordinator.qpl-tool-path <path to automata-dcap-qpl-tool> &
```

### 4. Start the TDX prover VM

A plain guest — see [above](#attestation-runs-in-dev-mode-and-the-tdx-guest-is-a-plain-vm) for why this must not be the real-TDX launcher in `hypervisor.nix`:

```bash
qemu-system-x86_64 -daemonize \
  -serial file:$PWD/tdx_prover.log -name guest=ethrex_tdx_prover \
  -machine q35,kernel_irqchip=split,hpet=off -smp 2 -m 2G \
  -accel kvm -cpu host -nographic -nodefaults \
  -bios /usr/share/ovmf/OVMF.fd -no-user-config \
  -netdev user,id=net0,net=192.168.76.0/24 -device e1000,netdev=net0 \
  -device ide-hd,bus=ide.0,drive=main,bootindex=0 \
  -drive "if=none,media=disk,id=main,file.filename=$PWD/image.raw,discard=unmap,detect-zeroes=unmap"
```

The VM has registered once the sequencer logs `ProverSetup received for TDX` without a following error, and the VM's serial console moves from `Error sending quote` to `No blocks to prove`.

### 5. Start the SP1 prover on `l2-gpu`

```bash
ssh l2-gpu
./ethrex-l2 l2 prover --backend sp1 \
  --proof-coordinators tcp://<tdx-host-tailscale-ip>:3900 --log.level info
```

### 6. Wait for a batch verified by both proofs

This is the assertion the test exists for. `lastVerifiedBatch` only advances once both an SP1 and a TDX proof have been submitted for the same batch.

```bash
while :; do
  C=$(rex call "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" 'lastCommittedBatch()' http://localhost:8545)
  V=$(rex call "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" 'lastVerifiedBatch()'  http://localhost:8545)
  echo "committed=$((C)) verified=$((V))"
  [ "$((V))" -ge 1 ] && break
  sleep 30
done
```

### 7. Run the integration suite

```bash
cd src
INTEGRATION_TEST_L1_RPC=http://localhost:8545 \
INTEGRATION_TEST_L2_RPC=http://localhost:1729 \
INTEGRATION_TEST_PRIVATE_KEYS_FILE_PATH=$PWD/fixtures/keys/private_keys_l1.txt \
INTEGRATION_TEST_BRIDGE_OWNER_PRIVATE_KEY=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
cargo test -p ethrex-test l2_integration_test --release --features l2 -- --nocapture --test-threads=1
```

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `execution reverted: 0x9e6e5638` on commit | `InvalidPrivilegedTransactionLogs()` — watcher interval left at the default; set `--watcher.watch-interval 1000` |
| `execution reverted` in `deploy-p256` | deploying onto a chain that already has the CREATE2 contracts; start from a fresh L1 datadir |
| `Source file requires different compiler version` | solc is not exactly 0.8.31 |
| VM loops on `Error sending quote: Failed to get ProverSetupAck` | the coordinator rejected registration; check the sequencer log for the error behind `Failed to handle ProverSetup` |
| `verifyBatch` reverts `0x62013a95` | `InvalidTdxProof()` — the VM was booted as a real TDX guest, so dev-mode `register()` stored quote-header bytes as the signer; use the plain-guest invocation |
| VM sits on `No blocks to prove` while batches are committed | the prover is asking for a batch whose input was pruned; restart from a fresh L1 **and** L2 with both provers attached before the first batch |
| VM sits on `No blocks to prove` with nothing committed | normal; if it persists, the committer is failing — check the sequencer log |

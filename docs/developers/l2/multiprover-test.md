# Multiprover test (SP1 GPU + TDX)

A per-release acceptance test for the configuration a real rollup runs: an `OnChainProposer` that requires **both** an SP1 proof and a TDX proof before it will verify a batch.

The other L2 checks each exercise one prover. This one is the only place where a batch has to satisfy two independent provers, running on two machines, before `lastVerifiedBatch` moves — and the only place a TDX quote is verified on chain, against real TDX hardware, rather than trusted.

## Real attestation, and what it needs

Deploy with `ETHREX_TDX_DEV_MODE=false` and boot the prover VM as a **real TDX guest**. `TDXVerifier.register()` then calls `verifyAndAttestOnChain`, so the quote is checked against the on-chain PCCS collateral and its measurements compared to the ones the verifier was deployed with.

Dev mode (`ETHREX_TDX_DEV_MODE=true`) is what CI uses, and it is not equivalent: `register()` short-circuits, taking the signing address from the quote's first 20 bytes and skipping verification entirely. That only works with a *plain* QEMU guest, whose dev quote begins with that address. Booting a real TDX guest in dev mode registers quote-header bytes as the signer and every `verifyBatch` then reverts with `InvalidTdxProof()` (`0x62013a95`) while registration appears to have succeeded. Dev mode is the fallback when no TDX host is available; it does not exercise the verifier.

Three things must be on chain before a real quote will verify. The deploy handles none of them, and `ethrex` normally loads the collateral by shelling out to `automata-dcap-qpl-tool` — which cannot work here, because it resolves contract addresses from a registry keyed by chain id and rejects the dev L1 with `Unsupported chain_id: 9`. Its failure is silent: `prepare_quote_prerequisites` discards the tool's exit status, so the first symptom is an unrelated revert later. Load the collateral directly instead, as [step 3](#3-load-this-platforms-tdx-collateral) does.

### The verifier's expected measurements

`_validateReport` requires MRTD and RTMR0-2 in the quote to equal the values compiled into `TDXVerifier.sol`. Those constants pin one specific image build, so they will not match the image of the release under test — a mismatch reverts with `MRTD mismatch`. Since the contracts are compiled at deploy time, pin them to the image being released, which is what this test should be asserting.

### The TCB Signing CA

The deploy upserts the root CA (`CA.ROOT`) and platform CA (`CA.PLATFORM`), but not `CA.SIGNING` — the Intel SGX TCB Signing certificate that signs the TCB info and QE identity. Without it the DAO has nothing to validate signatures against and rejects the upsert with `TCB_Cert_Expired` (`0xea8cd522`), which is misleading: nothing has expired.

### This platform's TCB info and QE identity

Fetched from the host's PCCS service and upserted into `AutomataFmspcTcbDao` and `AutomataEnclaveIdentityDao`.

## Where to run it

Two hosts, with the SP1 prover reaching the proof coordinator over the tailnet:

| Host | Runs | Requires |
| --- | --- | --- |
| `ethrex-tdx-baremetal` | L1, contract deploy, sequencer + proof coordinator, TDX prover VM | TDX-capable CPU, `qgsd` and `pccs` services, the nix-built TDX QEMU |
| `l2-gpu` | SP1 prover | CUDA GPU |

The TDX host really is required: a quote signed by real hardware is what `verifyAndAttestOnChain` checks. Confirm it before starting, because the VM boots on a non-TDX machine and only fails later, at quote generation:

```bash
cat /sys/module/kvm_intel/parameters/tdx     # must print Y
systemctl is-active qgsd pccs                # both active; pccs serves the collateral
```

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

### 2. Measure the image and pin the verifier to it

Boot the real TDX guest once, take a quote, and write its measurements into `TDXVerifier.sol` before the contracts are compiled. `run-qemu` from `hypervisor.nix` hardcodes `-serial mon:stdio`, so it dies when detached; the invocation below is the same TDX configuration with a file sink.

```bash
QEMU=$(nix-build --no-out-link crates/l2/tee/quote-gen/hypervisor.nix)/bin/run-qemu   # for reference
# daemonized equivalent:
qemu-system-x86_64 -daemonize -serial file:$PWD/tdx.log -name guest=ethrex_tdx_prover \
  -machine q35,kernel_irqchip=split,confidential-guest-support=tdx,hpet=off -smp 2 -m 2G \
  -accel kvm -cpu host -nographic -nodefaults -bios <OVMF from the nix store> -no-user-config \
  -netdev user,id=net0,net=192.168.76.0/24 -device e1000,netdev=net0 \
  -device ide-hd,bus=ide.0,drive=main,bootindex=0 \
  -drive "if=none,media=disk,id=main,file.filename=$PWD/image.raw,discard=unmap,detect-zeroes=unmap" \
  -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type":"vsock","cid":"2","port":"4050"}}'
```

The VM cannot reach a coordinator yet, so it logs `Error sending quote` — that is expected, and the quote it prints is what we need. Take it from the serial log and read the measurements out of it. In a v4 TDX quote the report body starts at byte 48; MRTD is at body offset 136 and RTMR0-2 follow at 328, 376 and 424, 48 bytes each.

```bash
QUOTE=$(grep -ao 'Sending quote [0-9a-f]*' tdx.log | head -1 | sed 's/Sending quote //')
python3 - "$QUOTE" <<'PYEOF'
import sys
b = bytes.fromhex(sys.argv[1])[48:]
for name, off in (("MRTD",136), ("RTMR0",328), ("RTMR1",376), ("RTMR2",424)):
    print(name, b[off:off+48].hex())
PYEOF
```

Replace the four `bytes public MRTD/RTMR0/RTMR1/RTMR2` initialisers in `crates/l2/tee/contracts/src/TDXVerifier.sol` with those values, then shut the VM down (`pkill -f 'guest=ethrex_tdx_prover'`) — it is started again in step 5, once the coordinator exists.

### 3. Start L1 and deploy with both provers required

```bash
./ethrex-linux-x86_64 --network src/fixtures/genesis/l1.json \
  --http.addr 0.0.0.0 --http.port 8545 --authrpc.port 8551 --dev --datadir dev_l1 &

cd src/crates/l2          # the deployer resolves tee/contracts relative to cwd
COMPILE_CONTRACTS=true ETHREX_TDX_DEV_MODE=false \
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

### 4. Load this platform's TDX collateral

The DAOs take `(string json, bytes signature)`. `cast` cannot pass these on the command line — its tuple parser splits on the commas inside the JSON — so encode the calldata and send it raw. Addresses come from `crates/l2/tee/contracts/deploydeps/automata-on-chain-pccs/deployment/`.

First the TCB Signing CA, which the deploy does not load. It is the leaf of the issuer chain the PCCS returns alongside the TCB info (`TCB-Info-Issuer-Chain` header, URL-encoded PEM):

```bash
cast send "$PCS_DAO" 'upsertPcsCertificates(uint8,bytes)' 3 "0x<signing cert DER>" \
  --private-key "$PK" --rpc-url http://localhost:8545
```

Then the TCB info and QE identity for this platform. The FMSPC is in the PCK certificate embedded in the quote, under OID `1.2.840.113741.1.13.1.4`:

```bash
curl -sk "https://localhost:8081/tdx/certification/v4/tcb?fmspc=$FMSPC" -o tcb.json
curl -sk "https://localhost:8081/tdx/certification/v4/qe/identity"      -o qeid.json
```

Each response is `{"<object>": {...}, "signature": "<hex>"}`; the DAO wants the inner object as a compact JSON string and the signature as bytes. Encode `upsertFmspcTcb((string,bytes))` and `upsertEnclaveIdentity(uint256,uint256,(string,bytes))` and send the calldata directly. For the identity, pass id `2` (`EnclaveId.TD_QE`) and version **4** — the DAO requires 4 or 5 for TD_QE and rejects the `version: 2` carried inside the JSON with `Incorrect_Enclave_Id_Version` (`0x4e0f5696`).

### 5. Start the sequencer

The proof coordinator must bind `0.0.0.0` so the SP1 prover on the other host can reach it, and the qpl tool path must be passed even in dev mode.

`--committer.commit-time` must not outpace proof generation. Both proofs are required per batch, so the pipeline advances at the slower prover's rate: an SP1 proof on `l2-gpu` measured a steady 106s, which is why 120s is used here. At 15s the committer outruns proving by about 7x, and the gap never closes — a 63-hour run reached batch 10983 committed against 2127 verified. Step 9 cannot pass in that state, however long it is left running.

```bash
set -a; . ~/multiprover/$TAG/.env; set +a
~/multiprover/$TAG/ethrex-l2-linux-x86_64 l2 --no-monitor \
  --watcher.block-delay 0 --watcher.watch-interval 1000 \
  --network ../../fixtures/genesis/l2.json \
  --http.addr 0.0.0.0 --http.port 1729 --datadir ~/multiprover/$TAG/dev_l2 \
  --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  --l1.on-chain-proposer-address "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" \
  --l1.timelock-address "$ETHREX_TIMELOCK_ADDRESS" \
  --eth.rpc-url http://localhost:8545 --committer.commit-time 120000 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.tdx-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.addr 0.0.0.0 \
  --proof-coordinator.qpl-tool-path <path to automata-dcap-qpl-tool> &
```

### 6. Start the TDX prover VM

Boot the real TDX guest again, exactly as in [step 2](#2-measure-the-image-and-pin-the-verifier-to-it) — the coordinator now exists, so the quote it sends is registered instead of erroring.

Registration has succeeded when the sequencer logs `ProverSetup received for TDX` followed by `ProverSetupACK sent`, with no `Failed to handle ProverSetup` between them. That ACK is the real result: with dev mode off, it means the quote passed `verifyAndAttestOnChain` against the collateral loaded in step 4 and its measurements matched the ones pinned in step 2.

### 7. Start the SP1 prover on `l2-gpu`

```bash
ssh l2-gpu
./ethrex-l2 l2 prover --backend sp1 \
  --proof-coordinators tcp://<tdx-host-tailscale-ip>:3900 --log.level info
```

### 8. Wait for a batch verified by both proofs

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

### 9. Run the integration suite

The withdrawal tests block in `wait_for_verified_proof` until `lastVerifiedBatch` reaches the batch holding the withdrawal, so verification has to be keeping pace before this starts. Confirm `lastVerifiedBatch` is climbing and within a few batches of `lastCommittedBatch` rather than thousands behind; if it is falling behind, fix the commit time in step 5 and restart from a fresh L1 and L2.

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
| `MRTD mismatch` on registration | the verifier was deployed with the committed measurements; pin them to the image under test (step 2) |
| `TCB_Cert_Expired` (`0xea8cd522`) on upsert | nothing has expired: `CA.SIGNING` was never loaded, so signatures cannot be checked (step 4) |
| `Incorrect_Enclave_Id_Version` (`0x4e0f5696`) | TD_QE needs version 4 or 5, not the `version` inside the JSON |
| `Unsupported chain_id: 9` | `automata-dcap-qpl-tool` cannot serve a dev chain; load the collateral directly (step 4) |
| `verifyBatch` reverts `0x62013a95` | `InvalidTdxProof()` — a real quote was registered while dev mode was on, so the signer is quote-header bytes; deploy with `ETHREX_TDX_DEV_MODE=false` |
| Suite hangs in a withdrawal test and later fails on an unreachable L2 RPC | verification is thousands of batches behind commits; `--committer.commit-time` is below the SP1 proof time (step 5) |
| VM sits on `No blocks to prove` while batches are committed | the prover is asking for a batch whose input was pruned; restart from a fresh L1 **and** L2 with both provers attached before the first batch |
| VM sits on `No blocks to prove` with nothing committed | normal; if it persists, the committer is failing — check the sequencer log |

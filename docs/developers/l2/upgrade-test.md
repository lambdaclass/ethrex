# Upgrade test

This is a per-release acceptance test. The goal is to verify that:

1. A node running the **previous release** (`<VERSION_FROM>`) can be cleanly stopped.
2. The L1 contracts can be upgraded following the per-release migration guide.
3. The **new release** (`<VERSION_TO>`) sequencer and prover resume operation against the upgraded contracts without re-deploying from scratch.

The commands below are ready to copy-paste. The only thing you need to substitute is the version tags. Every other value (private keys, addresses, ports, fee parameters) is taken from the defaults in `crates/l2/Makefile`; feel free to override them, but the values below give you a working baseline.

## Placeholders

Replace these once at the top of your shell session:

```bash
export VERSION_FROM=<previous release tag, e.g. v11.0.0>
export VERSION_TO=<new release tag, e.g. v12.0.0-rc.2>
export ARCH=linux-x86_64       # or linux-aarch64, macos-aarch64
export WORK=$HOME/upgrade-test  # workspace
mkdir -p "$WORK"
```

## Prerequisites

- [`rex`](https://github.com/lambdaclass/rex) on `$PATH` (used to call `upgradeToAndCall` and read contract state)
- `curl`, `jq`
- `git` (to fetch genesis / fixture files for each version)

## Reference docs (read these once)

- [Upgrade the contracts](../../l2/fundamentals/contracts.md#upgrade-the-contracts) — UUPS upgrade procedure.
- [Timelock](../../l2/fundamentals/timelock.md) — routing upgrades that hit `onlyOwner` on the OnChainProposer or `onlySelf` on the Timelock itself.
- [Upgrades](../../l2/deployment/upgrades.md) — **per-release migration steps**. This is the only document that changes between releases; the rest of this guide is constant.

## What is upgradable and what is not

### L1 contracts (UUPS proxies)

L1 contracts deployed behind a UUPS proxy can be upgraded in place. Their `_authorizeUpgrade` access control determines who can call `upgradeToAndCall`:

| Contract            | `_authorizeUpgrade` | Who upgrades it |
| ------------------- | ------------------- | --------------- |
| `OnChainProposer`   | `onlyOwner`         | The Timelock (must be routed through `schedule + execute` or `emergencyExecute`). Direct calls revert with `OwnableUnauthorizedAccount`. |
| `CommonBridge`      | `onlyOwner`         | The bridge owner directly (the address passed as `--bridge-owner` at deploy time). |
| `Router`            | `onlyOwner`         | The router owner directly. |
| `Timelock`          | `onlySelf`          | The Timelock itself — schedule + execute by Governance, or `emergencyExecute` by the Security Council. No EOA can call `upgradeToAndCall` directly. |
| `SequencerRegistry` (based only) | `onlyOwner` | The registry owner directly. |

### L2 system contracts (transparent proxies, upgraded from L1)

L2 system contracts (`CommonBridgeL2` at `0x...ffff`, `Messenger` at `0x...fffe`, `FeeTokenRegistry`, `FeeTokenPricer`, …) are pre-deployed in the L2 genesis as `TransparentUpgradeableProxy` with proxy admin set to `0x000000000000000000000000000000000000f000`. They **are** upgradable, but only from L1, by calling `CommonBridge.upgradeL2Contract(l2Contract, newImplementation, gasLimit, data)`:

```solidity
function upgradeL2Contract(address l2Contract, address newImplementation, uint256 gasLimit, bytes calldata data) public onlyOwner;
```

The bridge owner triggers a privileged L1→L2 transaction targeting `0x...f000`, which acts as the proxy admin and forwards `upgradeToAndCall` to the L2 proxy. The new implementation contract has to be deployed on L2 first (the `newImplementation` argument is an L2 address). See [Step 3.3 — L2 system contracts](#33-point-the-proxies-at-the-new-implementations).

> **Important:** because the L2 chain's state was initialized from the `$VERSION_FROM` L2 genesis, you **must not** restart the new sequencer with a different genesis file. The proxies at `0x...ffff`, `0x...fffe`, … are already in state; switching to the `$VERSION_TO` genesis would produce a divergent chain. The whole point of upgrading L2 system contracts on-chain is to avoid re-genesis. See [Step 0.1](#01-save-the-version_from-l2-genesis).

Check `docs/l2/deployment/upgrades.md` for which contracts each release actually changes — many releases only touch one of them.

---

## Step 0 — Workspace setup

Clone the source for both versions (you need the genesis file and the key fixtures from each). Then download the binaries.

```bash
cd "$WORK"
git clone --branch "$VERSION_FROM" --depth 1 https://github.com/lambdaclass/ethrex.git "ethrex-$VERSION_FROM"
git clone --branch "$VERSION_TO"   --depth 1 https://github.com/lambdaclass/ethrex.git "ethrex-$VERSION_TO"

# Binaries
curl -L "https://github.com/lambdaclass/ethrex/releases/download/$VERSION_FROM/ethrex-l2-$ARCH" -o "ethrex-$VERSION_FROM/ethrex"
curl -L "https://github.com/lambdaclass/ethrex/releases/download/$VERSION_TO/ethrex-l2-$ARCH"   -o "ethrex-$VERSION_TO/ethrex"
chmod +x "ethrex-$VERSION_FROM/ethrex" "ethrex-$VERSION_TO/ethrex"

"ethrex-$VERSION_FROM/ethrex" --version
"ethrex-$VERSION_TO/ethrex"   --version
```

> Use the `ethrex-l2-*` asset (the plain `ethrex-*` asset is an L1-only build and does not have the `l2` subcommand).

### 0.1 Save the `$VERSION_FROM` L2 genesis

The L2 chain is initialized **once**, from the genesis file shipped by `$VERSION_FROM`. The genesis embeds the runtime code of every L2 system contract at its fixed address (`0x...ffff`, `0x...fffe`, …). Once the chain is running, that state lives in the L2 datadir.

If `$VERSION_TO` changes any L2 system contract, its `fixtures/genesis/l2.json` will be different from `$VERSION_FROM`'s. **You must keep using the `$VERSION_FROM` genesis when starting the `$VERSION_TO` sequencer.** Pointing `--network` at the `$VERSION_TO` genesis would either fail the consistency check on startup or diverge from the existing chain. Upgrading the L2 system contracts in-flight (Step 3.3) is what reconciles the live state with the new bytecode — re-genesis is not.

Save it now so a future copy-paste of the Step 4 command can't accidentally point at the wrong file:

```bash
cp "$WORK/ethrex-$VERSION_FROM/fixtures/genesis/l2.json" "$WORK/l2-genesis-pinned.json"
```

The L1 genesis can stay pinned to `$VERSION_FROM` as well — the L1 process is started once at Step 1.1 and is **not** restarted or upgraded for the duration of this test.

---

## Step 1 — Run the `$VERSION_FROM` stack

### 1.1 Start L1 (Terminal A)

```bash
cd "$WORK/ethrex-$VERSION_FROM"
./ethrex \
  --network fixtures/genesis/l1.json \
  --http.port 8545 \
  --http.addr 0.0.0.0 \
  --authrpc.port 8551 \
  --dev \
  --datadir dev_ethrex_l1
```

### 1.2 Deploy L1 contracts (one-shot, in Terminal B)

This populates `cmd/.env` with the contract addresses; the sequencer reads them from there.

```bash
cd "$WORK/ethrex-$VERSION_FROM"
COMPILE_CONTRACTS=true ./ethrex l2 deploy \
  --eth-rpc-url http://localhost:8545 \
  --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
  --bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
  --bridge-owner-pk 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
  --deposit-rich \
  --private-keys-file-path fixtures/keys/private_keys_l1.txt \
  --genesis-l1-path fixtures/genesis/l1.json \
  --genesis-l2-path fixtures/genesis/l2.json \
  --env-file-path cmd/.env
```

After this finishes, capture the addresses we'll need later:

```bash
set -a; source "$WORK/ethrex-$VERSION_FROM/cmd/.env"; set +a
echo "BRIDGE   = $ETHREX_WATCHER_BRIDGE_ADDRESS"
echo "OCP      = $ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS"
echo "TIMELOCK = $ETHREX_TIMELOCK_ADDRESS"
```

### 1.3 Start the sequencer (Terminal B)

```bash
cd "$WORK/ethrex-$VERSION_FROM"
set -a; source cmd/.env; set +a
./ethrex l2 \
  --watcher.block-delay 0 \
  --network fixtures/genesis/l2.json \
  --http.port 1729 \
  --http.addr 0.0.0.0 \
  --datadir dev_ethrex_l2 \
  --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  --l1.on-chain-proposer-address "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" \
  --l1.timelock-address "$ETHREX_TIMELOCK_ADDRESS" \
  --eth.rpc-url http://localhost:8545 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
  --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
  --block-producer.operator-fee-per-gas 1000000000 \
  --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.addr 127.0.0.1 \
  --no-monitor
```

> `--l1.timelock-address` is required for non-based deployments from **v9 onwards** (the committer targets the Timelock, not the OCP directly). Without it the sequencer exits during startup. Drop the flag only if `$VERSION_FROM` predates v9; check the per-release migration guide for the exact set of flags.

> `--no-monitor` disables the terminal monitor UI. Keep it off for this test: the monitor renders a full-screen TUI that hides the startup logs, and it can deadlock node shutdown on an interactive terminal (see [#6911](https://github.com/lambdaclass/ethrex/issues/6911)). With `--no-monitor` the logs stream normally and `Ctrl-C` stops the node cleanly.

### 1.4 Start the prover (Terminal C)

```bash
cd "$WORK/ethrex-$VERSION_FROM"
./ethrex l2 prover \
  --proof-coordinators tcp://127.0.0.1:3900 \
  --backend exec
```

### 1.5 Confirm the stack is healthy

In a fourth terminal, wait until at least one batch has been committed:

```bash
rex l2 batch-number --rpc-url http://localhost:1729
```

Re-run until it returns a value `>= 1`. Then the `$VERSION_FROM` stack is working and we're ready to upgrade.

---

## Step 2 — Drain and stop the `$VERSION_FROM` sequencer and prover

Do **not** kill the sequencer first. Each committed batch is tagged with the sequencer's git commit hash, and the OCP looks the verification key up by that hash. If you stop the prover while there are still committed-but-unverified batches, the new prover (`$VERSION_TO`) will not be able to prove them — its commit hash is different.

The safe shutdown sequence is:

1. Tell the committer to stop accepting new batches (admin RPC).
2. Wait until every batch already committed has been verified (`lastCommittedBatch == lastVerifiedBatch` on the OCP).
3. Only then kill the sequencer and the prover.

L1 (Terminal A) stays up. L1 and L2 datadirs stay intact — the upgrade has to land on the same contracts and the same chain state.

> If you run the steps below from a fresh terminal, source the deployer env first so the address variables are set (otherwise `rex call` fails with `invalid value '' for '<TO>'`):
>
> ```bash
> set -a; source "$WORK/ethrex-$VERSION_FROM/cmd/.env"; set +a
> ```

### 2.1 Stop the committer

The admin server listens on `127.0.0.1:5555` by default; see [Admin API](../../l2/admin.md) for the full surface.

```bash
curl -sf -X GET http://localhost:5555/committer/stop
```

No new batches will be committed from this point on. Already-committed batches will still get proved and verified by the running prover.

### 2.2 Wait until all committed batches are verified

```bash
while :; do
  COMMITTED=$(rex call "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" 'lastCommittedBatch()' --rpc-url http://localhost:8545)
  VERIFIED=$(rex call  "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" 'lastVerifiedBatch()'  --rpc-url http://localhost:8545)
  echo "committed=$COMMITTED verified=$VERIFIED"
  [ "$COMMITTED" = "$VERIFIED" ] && break
  sleep 5
done
```

When the loop exits, the prover has caught up.

### 2.3 Kill the sequencer and the prover

In Terminal B (sequencer) and Terminal C (prover), send `Ctrl-C`. Or from another shell:

```bash
pkill -INT -f 'ethrex l2 --watcher' || true   # sequencer
pkill -INT -f 'ethrex l2 prover'    || true   # prover
sleep 2
sudo ss -tlnp | grep -E ':(1729|3900|5555)\b' || echo "L2 ports clean"
```

---

## Step 3 — Upgrade

### 3.1 Per-release migration

Open [`docs/l2/deployment/upgrades.md`](../../l2/deployment/upgrades.md) and follow the section that matches `$VERSION_FROM` → `$VERSION_TO`. That section will tell you exactly which of the following sub-steps apply.

Typical contents (varies per release):

- A **database migration** to run against the L2 store (SQL `ALTER TABLE`, table rename, …).
- A list of **contracts that need a new implementation**.
- **Post-upgrade calls** (e.g. v9 → v10 requires `setL2GasLimit` before unpausing the bridge).

### 3.2 Deploy the new implementations

#### Sanity check — diff the contract sources

The migration guide can lag behind the code. Before you trust it, diff the contract sources between the two checkouts. Any file that shows up here and is *not* listed in `docs/l2/deployment/upgrades.md` for this version bump is a gap to flag (and to upgrade by hand for this test):

```bash
diff -qr \
  "$WORK/ethrex-$VERSION_FROM/crates/l2/contracts/src" \
  "$WORK/ethrex-$VERSION_TO/crates/l2/contracts/src"
```

Group what comes out:

- Changes under `l1/` (e.g. `OnChainProposer.sol`, `CommonBridge.sol`, `Timelock.sol`, `Router.sol`) → L1 UUPS upgrade (Step 3.3, first three blocks).
- Changes under `l2/` (e.g. `CommonBridgeL2.sol`, `Messenger.sol`, `FeeTokenRegistry.sol`, …) → L2 transparent-proxy upgrade (Step 3.3, last block).
- Changes under `interfaces/` only → no on-chain action needed, but the binary's ABI changed; smoke-test the affected call paths.
- Changes under `based/`, `example/` → only relevant if you're testing the based deployment or examples; ignore otherwise.

If `diff` prints nothing, no contract upgrades are required for this release and you can skip the rest of Step 3.2 and Step 3.3.

#### Build and deploy

Use the `$VERSION_TO` source tree to build and deploy the new implementation bytecode. Only deploy the contracts the migration guide (or the diff above) tells you to.

```bash
cd "$WORK/ethrex-$VERSION_TO"
# Compile the contracts into local bytecode files (writes to target/.../solc_out/).
COMPILE_CONTRACTS=true ./ethrex l2 deploy --help >/dev/null  # touches the build script
# The compiled bytecode is embedded in the binary; to get standalone .bytecode files use forge/solc on
# the contract sources under crates/l2/contracts/src/l1/ — see the build script in cmd/ethrex/build_l2.rs.
```

For each upgradable contract listed in the migration guide:

```bash
# 1. Deploy the new implementation. Use the deployer's private key; this is just a code deployment.
rex deploy <NEW_IMPL_BYTECODE> 0 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --rpc-url http://localhost:8545
# rex prints the new implementation address — note it.
```

### 3.3 Point the proxies at the new implementations

The call depends on the contract's `_authorizeUpgrade`. See the table at the top.

**CommonBridge (`onlyOwner`, direct):**

```bash
rex send "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  'upgradeToAndCall(address,bytes)' <NEW_BRIDGE_IMPL_ADDR> 0x \
  --private-key 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
  --rpc-url http://localhost:8545
```

**OnChainProposer (`onlyOwner` → Timelock):** wrap the call in a Timelock `emergencyExecute` (fastest path; bypasses the delay). The Security Council key is the `--on-chain-proposer-owner` private key.

```bash
UPGRADE_CALLDATA=$(rex encode 'upgradeToAndCall(address,bytes)' <NEW_OCP_IMPL_ADDR> 0x)
rex send "$ETHREX_TIMELOCK_ADDRESS" \
  'emergencyExecute(address,uint256,bytes)' "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" 0 "$UPGRADE_CALLDATA" \
  --private-key <SECURITY_COUNCIL_PK> \
  --rpc-url http://localhost:8545
```

Production paths use `schedule(...)` then `execute(...)` after `minDelay`; the `emergencyExecute` path above is appropriate for this test because we want to keep it fast.

**Timelock itself (`onlySelf`):** same pattern as OCP, but with `target` = the Timelock proxy.

```bash
UPGRADE_CALLDATA=$(rex encode 'upgradeToAndCall(address,bytes)' <NEW_TIMELOCK_IMPL_ADDR> 0x)
rex send "$ETHREX_TIMELOCK_ADDRESS" \
  'emergencyExecute(address,uint256,bytes)' "$ETHREX_TIMELOCK_ADDRESS" 0 "$UPGRADE_CALLDATA" \
  --private-key <SECURITY_COUNCIL_PK> \
  --rpc-url http://localhost:8545
```

**L2 system contracts (transparent proxies, admin = `0x...f000`):** the implementation has to be deployed on **L2**, then the L1 `CommonBridge.upgradeL2Contract` sends a privileged tx that hits the proxy admin and forwards `upgradeToAndCall` to the proxy.

```bash
# 1. Deploy the new implementation on L2 (note: L2 RPC, L2 chain id, L2 funds).
#    The deployer must be funded on L2 — easiest is to deposit from L1 with rex first.
rex deploy <NEW_L2_IMPL_BYTECODE> 0 <L2_DEPLOYER_PK> --rpc-url http://localhost:1729
# rex prints the new L2 implementation address — note it.

# 2. From L1, ask the CommonBridge to upgrade the L2 proxy. Bridge owner key required.
#    <L2_CONTRACT_ADDR> is the L2 proxy address (e.g. 0xffff for CommonBridgeL2, 0xfffe for Messenger).
#    <DATA> is the calldata to run on the new implementation as initialization (use 0x if none).
rex send "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  'upgradeL2Contract(address,address,uint256,bytes)' \
  <L2_CONTRACT_ADDR> <NEW_L2_IMPL_ADDR> 1000000 <DATA> \
  --private-key 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
  --rpc-url http://localhost:8545
```

Wait until the privileged tx is consumed on L2 (look for a normal block production cycle to pass). To confirm the upgrade landed, read the ERC-1967 implementation slot of the L2 proxy:

```bash
curl -s http://localhost:1729 -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"eth_getStorageAt",
  "params":["<L2_CONTRACT_ADDR>","0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc","latest"]
}' | jq -r .result
```

The lower 20 bytes must equal `<NEW_L2_IMPL_ADDR>`.

### 3.4 Verify the proxy's implementation slot changed

For each proxy you upgraded:

```bash
curl -s http://localhost:8545 -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"eth_getStorageAt",
  "params":["<PROXY_ADDRESS>","0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc","latest"]
}' | jq -r .result
```

The lower 20 bytes must equal the new implementation address.

### 3.5 Post-upgrade calls

Apply any `setX` / `unpause` / `acceptOwnership` steps the migration guide lists. For example, v9 → v10:

```bash
rex send "$ETHREX_WATCHER_BRIDGE_ADDRESS" 'setL2GasLimit(uint256)' <NEW_GAS_LIMIT> \
  --private-key 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
  --rpc-url http://localhost:8545
```

---

## Step 4 — Run the `$VERSION_TO` stack

The L1, the L1 datadir, and the L2 datadir are all reused. The only thing that changes is the binary.

### 4.1 Restart the sequencer with the new binary (Terminal B)

```bash
cd "$WORK/ethrex-$VERSION_TO"
# Copy the addresses produced by the v$VERSION_FROM deployer.
cp "$WORK/ethrex-$VERSION_FROM/cmd/.env" cmd/.env
set -a; source cmd/.env; set +a
./ethrex l2 \
  --watcher.block-delay 0 \
  --network "$WORK/l2-genesis-pinned.json" \
  --http.port 1729 \
  --http.addr 0.0.0.0 \
  --datadir "$WORK/ethrex-$VERSION_FROM/dev_ethrex_l2" \
  --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  --l1.on-chain-proposer-address "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" \
  --l1.timelock-address "$ETHREX_TIMELOCK_ADDRESS" \
  --eth.rpc-url http://localhost:8545 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
  --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
  --block-producer.operator-fee-per-gas 1000000000 \
  --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.addr 127.0.0.1
```

Note two things:
- `--network` points at the **pinned `$VERSION_FROM` genesis** (Step 0.1). Never point this at `$VERSION_TO`'s genesis file, even if the L2 system contracts changed; those changes must be applied through Step 3.3, not by re-genesis.
- `--datadir` points back at the L2 store created under `$VERSION_FROM`.

### 4.2 Restart the prover (Terminal C)

```bash
cd "$WORK/ethrex-$VERSION_TO"
./ethrex l2 prover --proof-coordinators tcp://127.0.0.1:3900 --backend exec
```

---

## Step 5 — Acceptance check: run the integration tests against the upgraded L2

The simplest and strongest acceptance check is the standard L2 integration test suite (see [Integration tests](./integration-tests.md)), pointed at the **already-running** `$VERSION_TO` sequencer + prover from Step 4. The suite covers deposits, withdrawals, batch commit/verify, gas pricing and several other surfaces, so a green run subsumes the per-check list we used to have here.

Run it from the `$VERSION_TO` source tree so the tests match the binary you just upgraded to:

```bash
cd "$WORK/ethrex-$VERSION_TO"
cp "$WORK/ethrex-$VERSION_FROM/cmd/.env" cmd/.env   # contract addresses from the v$VERSION_FROM deploy
cd crates/l2
make test
```

`make test` calls `cargo test -p ethrex-test l2_integration_test --features l2 -- --nocapture`. It assumes the dev node and prover are already up on the default ports (`L2 RPC 1729`, `proof coordinator 3900`), which is exactly the state Step 4 leaves you in.

See [Integration tests › I think my tests are taking too long, how can I debug this?](./integration-tests.md#i-think-my-tests-are-taking-too-long-how-can-i-debug-this) for what to do if it stalls.

If the suite goes green, the upgrade is successful.

If it fails, capture:
- the migration step you stopped at,
- the proxy implementation slot read (Step 3.4),
- the first 100 lines of the sequencer log after the Step 4 restart,
- the failing test name from the `make test` output.

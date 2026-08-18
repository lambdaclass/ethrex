# Hegotá testnet — upgrading a running deployment

How to change the software on a live Hegotá testnet host without a re-genesis, and
when a re-genesis is unavoidable.

The governing constraint: **a re-genesis changes the genesis hash, which invalidates
every published artifact and every joiner's database.** Anyone following the chain has
to wipe and re-sync, and the bootnode list, `genesis.ssz` and
`deposit_contract_block_hash.txt` all change. Treat it as launching a new network, not
as an upgrade. Everything below is arranged so the common cases avoid it.

Companion documents: `docs/hegota-testnet-joining.md` (what joiners consume),
`docs/hegota-testnet-permissioning.md` (validator gating),
`docs/hegota-testnet-verification.md` (the twelve-check verification pass).

## Before touching anything

Record the head and a deep block hash, so an accidental re-genesis or database rewrite
is caught immediately rather than discovered by a joiner:

```
RPC=http://127.0.0.1:32003
cast block-number --rpc-url $RPC
cast block 1000 --rpc-url $RPC --json | jq -r .hash
cast rpc debug_chainConfig --rpc-url $RPC | jq '{amsterdamTime, hegotaTime}'
```

Re-run all three after the change. The block-1000 hash must be identical. A changed
genesis or fork schedule means you have a different chain, whatever the head says.

## Upgrading ethrex

**Changing `el_image` and re-running the kurtosis config does not upgrade a running
node.** Kurtosis does not recreate containers that already exist, so the tag reported by
`docker ps` describes what the container was created from, not what is executing. Always
read the version from the binary itself:

```
docker exec <el-container> ethrex --version
```

Which upgrade path applies depends on how much the chain can afford to lose, and that
changes the moment the network has outside participants:

- **Before anyone else has joined**, the cheapest correct upgrade is a re-genesis: tear
  the enclave down, rebuild the image, launch again. Nothing external depends on the
  chain yet, so there is no reason to preserve it.
- **Once joiners exist**, a re-genesis is a relaunch of a different network (see below),
  so the binary has to be replaced inside the running containers. That preserves each
  node's datadir and node key, which is what keeps the published `bootnodes.txt` valid
  and spares every joiner a re-sync.

The second path needs to be set up deliberately at install time, because swapping a
binary under a running process is only simple if the entrypoint was built for it. The
pattern the Hegotá devnet uses, and the one to copy if we want this: make the container
entrypoint a small wrapper that execs a separate binary file, so an upgrade replaces that
file and restarts the process rather than the container. Decide this in
`scripts/hegota-testnet/INSTALL.md` before launch; retrofitting it later means recreating
the containers, which is the thing it exists to avoid.

Whichever path, do one node at a time and wait for it to rejoin before starting the next.
Two of three must stay up: the chain needs its validators attesting, and each consensus
client is pinned to its own execution client.

**A consensus-rule change is not an in-place upgrade.** If the new binary changes what
counts as a valid block at a timestamp already passed, the upgraded node disagrees with
its own history. Consensus changes need a re-genesis, or an activation timestamp far
enough in the future that every node adopts the binary before it arrives.

## Changing a node's flags in place

Adding, removing or retuning an ethrex flag on a live node — a mempool knob, a log
level, an RPC namespace — is not a binary upgrade, but it needs the same care, because
the flag lives in the container's command line and a command line cannot be edited.

Three obvious routes do not work, all three verified against kurtosis 1.20.0 in a
throwaway enclave rather than assumed:

- **There is no runtime setter.** The `admin` namespace exposes `admin_setLogLevel` and
  nothing else; no mempool or RPC option can be changed over JSON-RPC.
- **Re-running the kurtosis config does nothing.** Kurtosis does not recreate a
  container that already exists, which is the same reason `el_image` changes do not
  upgrade a running node.
- **`kurtosis service update --cmd` is worse than useless here.** It destroys the
  container — new service UUID, old container removed, so the datadir in the writable
  layer goes with it — *and* it passes the whole `--cmd` string as a single argv
  element, so the replacement exits 127 immediately. Space-separated and
  comma-separated forms both fail. A multi-argument command line cannot be expressed
  through it.

What works is recreating the container from a snapshot of itself. `docker commit`
captures the writable layer, so the chain database and `chain-8141/node.key` come
across, and with the node key the enode — which matters because all three enodes are
published in `bootnodes.txt`.

Per node, one node at a time, with the faucet's node last (it submits through the RPC
on el-1, so that node's window is the only one users can see):

```
# 1. record what must not change
curl -s -X POST -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}' \
  http://127.0.0.1:<rpc-port> | jq -r .result.enode

# 2. stop, snapshot, recreate with the same identity and the new flag appended
scripts/hegota-testnet/set-el-flags.py <container> <rpc-port> -- --some.flag=value

# 3. verify before touching the next node, then drop the stale container
docker rm <container>.old
```

The script derives everything from `docker inspect`, so nothing is hand-transcribed:
name, hostname, network, static IP, network aliases, every label (including the
`traefik.*` routing labels), the published host ports, the `/network-configs` and
`/jwt` volume binds, the restart policy and the entrypoint. `docker commit` carries the
image config — env, entrypoint, exposed ports — so only the command line changes.

Three things to know before running it:

- **Remove the stale `.old` container once the new one is verified.** Until then two
  containers carry the same `com.kurtosistech.guid` label and
  `kurtosis enclave inspect` reports the service **STOPPED** while it is in fact
  running. `publish-artifacts.sh` parses that table.
- **`docker ps` will report the snapshot image**, `mvg500-snapshot:…` or whatever tag
  the run used, not `ethrex:hegota-testnet`. The binary is unchanged; read the version
  from `docker exec <el> ethrex --version`, which this document already insists on for
  the same reason. The snapshot image cannot be deleted while the container it created
  runs, and it doubles as the rollback path: recreate from it without the new flag.
- **Confirm the stop was clean.** The script prints the exit code; `0` means ethrex
  handled SIGTERM and closed its store. A `137` means it was killed at the end of the
  grace period and the database may need recovery on start.

Verify, per node, before moving on: the container is running, `eth_blockNumber`
advances and agrees with the other two, `eth_syncing` is `false`, `net_peerCount` is
back to its previous value, the enode is byte-identical to the one recorded in step 1,
the flag is in `docker inspect -f '{{json .Config.Cmd}}'`, and
`kurtosis enclave inspect` shows the service RUNNING once the `.old` container is gone.

A flag on a command line is not proof that the node is using the value, and for
`MAX_VERIFY_GAS` the node will report it. EIP-8141 mempool rule #6 rejects a frame
transaction whose signature-verification cost alone exceeds the budget, before any
crypto runs — so junk signatures reach the gate, and the rejection quotes the
configured limit. P256 signatures cost 6700 each, so build two transactions with
`scripts/hegota-testnet/frametx.py` (75 and 74 P256 signatures, junk bytes, signer set,
empty msg) and simulate each:

```
curl -s -X POST -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","method":"ethrex_simulateFrameTransaction","params":["<raw>"],"id":1}' \
  https://rpc1.privacy.ethrex.xyz | jq -r .result.violation
```

At 500 000 the 75-signature transaction answers `signature verification cost 502500
exceeds MAX_VERIFY_GAS 500000` and the 74-signature one gets past the gate to `frame
signature list does not authenticate the sender`. A node still at the default quotes
`100000` and refuses both. `ethrex_simulateFrameTransaction` is read-only and the
`ethrex` namespace is open through the public guard, so this works against each
`rpc{1,2,3}.privacy.ethrex.xyz` endpoint without shell access to the host.

This preserves the database, so there is no re-sync and the only cost is the seconds
the node is down — but it is still a per-node action on a chain with outside
participants. Update the deployment's args-file on the host **and** the committed
`fixtures/networks/hegota-testnet.yaml` in the same session, or the next relaunch
silently reverts the change.

## Upgrading kurtosis

**The CLI and the engine must be the same version.** The engine exposes an API version
and the CLI refuses to talk to a mismatched one:

```
An API version mismatch was detected between the running engine version '1.18.2'
and the engine version the CLI expects, '1.20.0'
```

So upgrading the CLI forces `kurtosis engine restart`, which replaces the engine
container. Plan it as a maintenance action, not a side effect of a package upgrade.

Releases are published at
`github.com/kurtosis-tech/kurtosis-cli-release-artifacts`, one tag per version, with
`.deb`, `.rpm` and `.tar.gz` for each platform. The `apt.fury.io/kurtosis-tech` channel
is stale — it stops at 1.15.2 — so do not expect `apt upgrade` to move the version.

```
gh release download <version> --repo kurtosis-tech/kurtosis-cli-release-artifacts \
  --pattern 'kurtosis-cli_<version>_linux_amd64.tar.gz'
tar xzf kurtosis-cli_<version>_linux_amd64.tar.gz
install -m755 kurtosis /usr/local/bin/kurtosis
kurtosis engine restart
kurtosis enclave inspect hegota-testnet
```

Verify the enclave's services are all `RUNNING` and re-run the head checks afterwards.
`kurtosis enclave inspect` has **no machine-readable output mode** in any release
through 1.20.0 — the only flags are `--full-uuids` and `--help` — so any tooling that
parses it reads the human-readable table. `publish-artifacts.sh` does exactly that.

## Re-publishing the artifact bundle

Re-run after anything that changes a node's identity or the genesis:

```
PUBLIC_IP=<host public ip> ENCLAVE=hegota-testnet OUT_DIR=/srv/hegota-testnet/artifacts \
  ./scripts/hegota-testnet/publish-artifacts.sh
```

It publishes an explicit allowlist, not the generator's whole output directory. That is
deliberate: `/network-configs` also contains `mnemonics.yaml`, the BIP-39 phrase every
genesis validator key derives from, and `OUT_DIR` is served publicly. Never replace the
allowlist with a directory copy, and never add a file to it without adding a row to the
artifact table in `docs/hegota-testnet-joining.md`.

If a node's enode changed (new node key, new datadir, new advertised IP), the old
`bootnodes.txt` sends joiners at a peer that no longer exists. Re-publish and announce.

## Rotating the deposit-gater admin

A genesis admin is granted with storage value `2`, which `SimpleAccessControl` treats as
sticky, and `revokeRole` refuses to revoke it. **A genesis admin cannot be removed for
the life of the chain.** The remedies are:

- grant a runtime admin (`grantAdmin`, written as `1`) and use that one day to day,
  keeping the genesis key offline and unused;
- if a key leaks, block the affected prefixes (`setConfig --prefix 0x01 --blocked true`),
  which stops new validators of that type immediately without touching anyone's tokens;
- re-genesis, if the leak cannot be contained by the kill switch.

Plan for this before launch by keeping the genesis admin cold.

## What forces a re-genesis

- a change to the genesis allocation, the fork schedule, or the chain ID;
- a consensus-rule change effective at a timestamp already in the past;
- a leaked genesis gater admin that the per-prefix kill switch cannot contain;
- a leaked validator mnemonic, since every genesis validator key derives from it.

When one is unavoidable, treat it as a new launch: fresh keys from
`scripts/hegota-testnet/gen-deployment-keys.sh`, a fresh enclave, the full twelve-check
pass from `docs/hegota-testnet-verification.md`, a re-published bundle, and an announcement
that the old chain is dead. Do not reuse the previous deployment's mnemonics.

# Hegotá Devnet — Upgrade Guide

How to upgrade the devnet's execution client without breaking the chain. Every
procedure here was validated on a live enclave before being written down; the
failure modes are ones we reproduced deliberately.

## The one invariant that decides everything

ethrex regenerates state on startup by **re-executing recent blocks** (from the
last flushed state layer to head) under the rules of the *currently running
binary*, selected per block timestamp. State is flushed to disk only every
`DB_COMMIT_THRESHOLD` (128) blocks, so a restart always replays a tail of the
chain.

> **Consequence: a binary that changes the state transition of blocks that
> already exist can never run on that chain.** Re-execution computes state
> roots that differ from the ones sealed in the headers, and the node exits
> with `Invalid Block: World State Root does not match`.

This is not an ethrex quirk — it is how Ethereum upgrades work everywhere:
rules for the past are frozen; new rules activate at a scheduled future fork.

### The litmus test

Before any upgrade, ask of the diff:

> *Would re-executing an already-produced block under the new binary yield a
> different state root, receipt, or gas usage?*

- **No** → the change is non-consensus. Path 1 (binary swap) or, if the change
  is in the container command rather than the binary, Path 1b (wrapper).
- **Yes, but only for a fork the chain has NOT reached** → Path 2
  (deploy before the fork).
- **Yes, for rules the chain has ALREADY been running** → Path 2's new-fork
  decoupling variant (a future successor fork / config knob). **Re-genesis is
  FORBIDDEN on this devnet** (see below) — history is never wiped.

Be paranoid with this test. Changes that *look* operational can be consensus:

- Gas accounting and refunds (a frame can observe the payer's balance
  mid-transaction — charging a different amount at APPROVE is
  consensus-visible even when the net end-of-tx balance is identical).
- Predeploy installs (they execute at a fork boundary and produce an account
  update in the state root).
- Anything touching `total_gas_limit`, intrinsic gas, opcode gas, journaling,
  or the BAL (EIP-7928) footprint.

Genuinely non-consensus: RPC handlers, mempool admission policy (execution
still validates in full — e.g. `MAX_VERIFY_GAS` is a mempool-admission bound
only, never checked in block execution), P2P, logging, docs, the payload
*builder's* tx selection (not its execution).

### Answer the test against the chain's actual history

The test is about blocks that **already exist**, so a consensus change to a
feature this chain never used is Path 1 material. Enumerate before deciding
rather than reasoning from the diff alone — for a change confined to frame
transactions, walk the chain and inspect the frame modes:

```bash
# every type-0x06 tx in the chain, with its frame modes
for ((s=0; s<=HEAD; s+=200)); do
  e=$((s+199)); (( e>HEAD )) && e=$HEAD
  req="["; for ((b=s; b<=e; b++)); do
    req+="{\"jsonrpc\":\"2.0\",\"id\":$b,\"method\":\"eth_getBlockByNumber\",\"params\":[\"$(printf '0x%x' $b)\",true]},"
  done
  curl -s -X POST -H 'content-type: application/json' -d "${req%,}]" "$RPC" \
   | jq -c '.[].result | select(.!=null)
            | {n:.number, frames:[.transactions[]|select(.type=="0x6")|{h:.hash, modes:[.frames[].mode]}]}
            | select(.frames|length>0)'
done
```

Only the replay tail is re-executed on restart — from the last flushed state
layer (every `DB_COMMIT_THRESHOLD` = 128 blocks) to head — but treat the whole
history as in scope: any node syncing from genesis replays all of it.

A worked example: the EIP-7906 transaction-prestate fix changes what `TXDIFF`
returns, which is consensus-visible, yet it only affects transactions carrying a
POST_TX frame. Enumerating showed the chain's history held two frame
transactions, both VERIFY+SENDER, so no already-produced block could change and
Path 1 applied. The three ELs restarted with zero
`World State Root does not match`, confirming it.

> **Never use `kurtosis service update` for a state-preserving upgrade.** It
> recreates the container, which drops the file mounts (`/network-configs`,
> `/jwt` → `Failed to open genesis file`) and destroys the EL datadir (it lives
> in the container **writable layer**, not a volume) → resync from genesis.
> Never recreate the container — re-genesis is FORBIDDEN on this devnet (below),
> and Paths 1 and 1b never recreate it.

## Path 1 — In-place binary swap (non-consensus binary changes)

Zero downtime, zero history loss, no container recreate. The default for any
non-consensus change that lives in the **binary** (RPC handlers, mempool policy,
P2P, logging, builder selection).

Why it is safe: `docker restart` re-runs the entrypoint against the **same
writable layer**, so overwriting the binary in place and restarting leaves the
datadir physically untouched; the node re-executes the block tail and — for a
non-consensus change — computes identical state roots.

1. Build under a **unique immutable tag** (never reuse a mutable tag like
   `ethrex:hegota` on a shared build host — a concurrent build can hijack it):
   `make build-image TAG=hegota-<short-sha>`, then verify
   `docker run --rm --entrypoint ./ethrex ethrex:hegota-<short-sha> --version`.
2. Extract the binary once:
   `id=$(docker create ethrex:hegota-<short-sha>); docker cp $id:/usr/local/bin/ethrex /tmp/ethrex-new; docker rm $id`.
   **If you `scp` the binary between hosts, `chmod +x /tmp/ethrex-new` afterwards** —
   `scp` drops the execute bit and the container crash-loops
   `exec: /usr/local/bin/ethrex-real: Permission denied` (`docker cp` from an
   image preserves it). The datadir is untouched because the failure is pre-exec.
3. **Canary order: el-2 first (never the bootnode), then el-3, then el-1.**
   **Exception — a consensus change (even a history-safe one) must not be
   canaried.** While the versions are mixed, an upgraded builder can produce a
   block the un-upgraded nodes consider invalid, splitting the set. Stage the
   binary into *every* EL first (`docker cp` to all of them, no restarts), then
   restart them back to back, and verify only afterwards. The window where a
   POST_TX-carrying transaction could be included is what you are minimising.
4. **Check what `/usr/local/bin/ethrex` actually is first.** If Path 1b has ever
   been applied to this enclave, that path is a ~240-byte shell **wrapper** that
   appends flags and execs the real binary at `/usr/local/bin/ethrex-real`. Copying
   the new binary over `ethrex` then silently discards the wrapper, and the flags it
   added (for us, `--http.api=…,ethrex`) vanish on the next restart — the RPC
   namespace disappears and the cause is three steps back. Confirm before copying:

   ```bash
   docker exec <ctr> ls -la /usr/local/bin/ | grep ethrex
   # ethrex (239)  ethrex-real (41895680)   <- wrapper present: target ethrex-real
   # ethrex (41895680)                      <- no wrapper: target ethrex
   ```

   The size is the tell — a wrapper is bytes, a binary is tens of megabytes. Save
   both before overwriting either, and put the new binary where the *old binary*
   was, not where the entrypoint points.

5. Per EL: `docker cp /tmp/ethrex-new <ctr>:/usr/local/bin/ethrex-real` (or
   `…/ethrex` if there is no wrapper), `docker exec <ctr> chmod +x` it, then
   `docker restart -t 20 <ctr>`.
6. **Host ports:** enclaves with `port_publisher` set have **deterministic**
   ports that survive restart. Enclaves that publish dynamically (older kurtosis)
   **remap the host port on every restart** — re-read it with
   `docker port <ctr> 8545` (or `kurtosis port print`) before verifying.
7. Verify (checklist below) before touching the next EL.
8. Rollback: keep the previous image (`docker tag ethrex:<current> ethrex:rollback-<sha>`)
   so you can `docker cp` the old binary back and restart.

**Durability:** the swapped binary lives in the writable layer — it survives
reboots (set `docker update --restart unless-stopped` on all containers; the ELs
do **not** ship with a restart policy, so check
`docker inspect --format '{{.HostConfig.RestartPolicy.Name}}'` rather than
assuming) but a container **recreate** reverts to the image. Retag the image
(`docker tag ethrex:hegota-<short-sha> ethrex:hegota`) so a future recreate uses
the new binary.

When the image was built on a *different* host from the one running the enclave,
a retag is not enough — the devnet host's image still holds the old binary, and
tagging cannot conjure the new one. Either build on the devnet host, or ship the
image itself (`docker save … | ssh … docker load`) before retagging. Until that
is done, the swap is restart-durable but recreate-fragile: the running chain has
the fix and the image does not.

## Path 1b — Add/change a CLI flag or RPC namespace without re-genesis (wrapper)

For a **non-consensus** change that lives in the container **command**, not the
binary — e.g. exposing a new RPC namespace via `--http.api`, or any flag tweak.
The command is baked into `.Config.Cmd` at container creation; changing it
normally needs a recreate (datadir loss). Wrap the entrypoint instead — same
`docker cp` + `docker restart` mechanics as Path 1, so it is equally
state-preserving, per-EL, and reversible.

1. Swap the binary to a **sidecar path** (Path 1, but as `ethrex-real`):
   `docker cp /tmp/ethrex-new <ctr>:/usr/local/bin/ethrex-real`.
2. Install a wrapper at the **entrypoint path** that re-execs the real binary
   with the extra flags appended:
   ```
   printf '#!/bin/sh\nexec /usr/local/bin/ethrex-real "$@" --http.api=eth,net,web3,ethrex\n' > /tmp/w
   chmod +x /tmp/w
   docker cp /tmp/w <ctr>:/usr/local/bin/ethrex
   ```
   Put the wrapper where the image's ENTRYPOINT resolves: `["ethrex"]` is
   PATH-resolved (`/usr/local/bin/ethrex`); `["./ethrex"]` is relative to the
   image `WorkingDir` (check `docker inspect <ctr> --format '{{.Config.WorkingDir}}'`).
3. `docker restart -t 20 <ctr>`, then verify, canary order el-2 → el-3 → el-1.

**The `--http.api` union rule.** `--http.api` is a multi-value flag with
`args_override_self`, so multiple occurrences **union** (accumulate), not
override. Appending `--http.api=ethrex` therefore *merges* `ethrex` into whatever
the launcher already set. **But if the container passes no `--http.api`** (it then
runs the default `eth,net,web3`), your appended occurrence is the *only* one — so
append the **full** set you want (`eth,net,web3,ethrex`), or you will drop the
defaults. Always check first:
`docker inspect <ctr> --format '{{json .Config.Cmd}}'`.

**Reversible:** `docker cp` the real binary back over the wrapper and restart.
**Durability:** retag the image (Path 1) **and** bake the flag into the launcher
`ethereum-package/src/el/ethrex/ethrex_launcher.star` too (defence-in-depth if a
container is ever rebuilt from the image). The wrapper lives in the writable
layer: it survives restarts and reboots and is lost only on a container
*recreate* — which this devnet never does (re-genesis is forbidden), so in
practice the wrapper is the durable mechanism.

## Path 2 — Deploy before the fork (the mainnet-standard upgrade)

For consensus changes gated on a fork timestamp the chain has **not** reached.

1. Ensure every new rule is gated `fork >= NewFork` (predeploy installs must be
   idempotent and gated the same way — they re-run on every block during
   re-execution, which is what makes replay safe).
2. Schedule the fork **in the future** in the genesis/chain config. Setting a
   fork time at or before existing block timestamps retroactively redefines
   history → guaranteed state-root mismatch.
3. Swap all ELs (Path 1 binary-swap mechanics) **while the chain is still
   pre-fork**. The new binary re-executes pre-fork blocks under pre-fork rules —
   identical results — and the fork later activates under the new binary.
4. If the fork also needs CL awareness (new engine-API version, new payload
   fields), confirm the CL release supports it *before* scheduling.

### Variant: new-fork decoupling (history-preserving change to active behavior)

If the chain already crossed fork F and you must change behavior that F
introduced, do **not** redefine F. Gate the change on a new config knob /
successor fork F′ set in the future (precedent: the `postHegotaTime` field
that deferred the NONCE_MANAGER install on an already-post-Hegotá chain).
This preserves history at the cost of a non-canonical fork layout. **On this
devnet it is the REQUIRED path** for changing already-active behavior, because
re-genesis is forbidden (below). Fresh chains must keep working with the knob
unset (fall back to F).

### Variant: a config field the genesis generator cannot emit

Some behavior is gated on a chain-config field `ethereum-genesis-generator` knows
nothing about (EIP-8312's `utxoFramesTime` is the current example). Path 2 applies
unchanged — schedule it in the future, swap binaries while still pre-activation —
plus one step: write the field into each EL's `/network-configs/genesis.json` after
the enclave is up, then restart that EL.

Three things make this step go wrong quietly rather than loudly:

- **Patch the JSON on the host** (`docker cp` out, edit, `docker cp` back). The EL
  containers have no Python. And a `sed` anchored on a fork name is a trap: the
  execution config calls the Hegotá fork `bogotaTime` — `heze` is only the
  consensus-layer name — so anchoring on `hezeTime` matches nothing while the
  command still reports success.
- **Select containers by enclave label**, `--filter label=com.kurtosistech.enclave-name=<enclave>`.
  Kurtosis container names share the `el-N-<el>-<cl>` prefix across enclaves and
  differ only by a UUID suffix, so a name-prefix filter can silently patch a
  different network's nodes.
- **The field is invisible to ForkId**, so there is no peer-level protection at the
  boundary: every EL needs both the new binary and the patched config before the
  timestamp. An un-patched node degrades by rejecting the new transaction shape, never
  by rewriting history — so the failure looks like "the feature doesn't work" rather
  than a split.

## FOCIL (EIP-7805): the one upgrade that is not EL-only

Every other change in this guide is state-preserving on the execution side and
invisible to the consensus client. FOCIL is not, and it does not fit Path 1, Path
1b or Path 2.

EIP-7805 activates at `Fork::Hegota`, which the consensus layer calls `heze`.
There is no separate config knob and deliberately so: `hegota_time` is derived
from `heze_fork_epoch` by `ethereum-genesis-generator`, so the two layers already
share one activation point, and a second timestamp would create two clocks for one
fork with a halt window between them.

**Both layers must move together.** From Hegotá on, an EL carrying FOCIL returns
`UnsupportedFork` for `engine_newPayloadV5` and `engine_forkchoiceUpdatedV4`,
because only V6/V5 carry `inclusionListTransactions`. Hegotá is long past on this
devnet, so:

- new EL under the current client → that node demands V6, the client speaks only
  V5/V4, the node stops importing and building;
- new client under the current EL → the client demands V6, the EL does not
  advertise it, same outcome.

There is no inert intermediate state and no ordering that avoids one. Consensus
clients have no fallback either: every branch is `if capability { call } else {
Err(RequiredMethodUnsupported) }`.

### The consensus side is already scheduled

`/network-configs/config.yaml` on the live devnet already carries
`HEZE_FORK_VERSION: 0x90000038` and `HEZE_FORK_EPOCH: 2`. The running
`ethpandaops/lighthouse:glamsterdam-devnet-7` is built from a branch whose
`ForkName` stops at `Gloas`, so it parses neither and `/eth/v1/config/spec`
reports no heze at all — which is exactly why a stock client has been driving the
frame-transaction stack this whole time.

So this needs **no re-genesis**, only a heze-aware image. Note the epoch is long
past (heze is epoch 2, the chain is past epoch 3400), so a FOCIL-capable client
enters heze the moment it starts; there is no scheduling margin to work with.

### What to move to

The image is **`ethpandaops/lighthouse:focil`**, built from `sigp/lighthouse`
branch `focil`. The fallback is `ethpandaops/teku:focil`, built from
`Consensys/teku` branch `prototype/focil`. Both are auto-built: the source of
truth is `ethpandaops/eth-client-docker-image-builder`, file `branches.yaml`, key
`cl.clients.<client>.branches`, which lists `focil` for both today. Replace
`cl_image: ethpandaops/lighthouse:glamsterdam-devnet-7` with the `:focil` tag.

`sigp/lighthouse@focil` is a strict superset of `unstable` (the branch the
devnet-7 images build from) on every axis that matters: the same `ForkName` list
through `Gloas` and `Heze`, `JsonPayloadAttributesV4` and `V5` both carrying
`slot_number` and `target_gas_limit`, and the engine set extended with
`forkchoiceUpdatedV5`, `newPayloadV6` and `getInclusionListV1`. Teku's
`prototype/focil` matches on all of it. A newer stock build is not a substitute:
`unstable` rejects a Heze payload outright with `UnsupportedForkVariant`.

What is unproven is whether either handles the rest of the devnet-7 network
config, since both branches sit ~100+ commits behind their own master. Branch
dates do not settle it. Measure on a scratch enclave, never on the live chain.

Check the tag's build date before planning around it: as of 2026-08 the images
are `lighthouse:focil` from 2026-06-16 and `teku:focil` from 2026-06-25, both
predating glamsterdam-devnet-7. Because the images track their branches, the
lever for a fresher one is getting the upstream branch rebased, not a rebuild
request. Teku's is the cheaper ask, being only ~7 commits ahead of its own
master.

### Before attempting this

Two things are unresolved and must be settled first, not discovered mid-upgrade:

- **The client swap procedure itself.** Path 1 covers ELs. Replacing a running
  consensus container is a different operation, and the CL genesis state is frozen
  because re-genesis is forbidden.
- **The rollback.** Because both layers move at once and Hegotá is already active,
  reverting means reverting both. Establish that path before starting.

## Re-genesis — FORBIDDEN

Re-genesis (`kurtosis enclave rm -f` + `kurtosis run`) wipes all chain state —
balances, deployed contracts, history, users' funded accounts — and breaks
everyone building on the public devnet. **It is not permitted on this devnet
under any circumstances.** Preserve state, always.

- Consensus change to an already-active fork → use the new-fork decoupling
  variant above (a future successor fork / config knob). Never a re-genesis.
- **CL fork-epoch / CL genesis params** are the one thing that would technically
  require re-genesis (they live in the CL genesis state). Because re-genesis is
  forbidden, CL fork config is effectively **frozen**: any change that needs it
  must be **escalated to the devnet owner** and never performed unilaterally.
- Everything else uses Path 1 (binary swap) or Path 1b (wrapper) — both
  state-preserving; neither recreates the container.

## Runbook: activating EIP-8312 on the live devnet

EIP-8312 is scheduled on its own config field, `utxoFramesTime`, so it takes
Path 2 plus the generator-cannot-emit variant above. What follows is only what is
specific to it; the general checklist below still applies in full.

### The rollback deadline

**Rollback stops being possible the moment the timestamp passes.** Before `T` the
change is inert and a node can be reverted to a binary without EIP-8312 freely.
From the first block at or after `T`, blocks carry the vault install and the
openings-root writes, and a binary that does not implement them recomputes
different state roots for that history — the node exits with
`Invalid Block: World State Root does not match` and cannot resync. Re-genesis is
forbidden here, so there is no recovery.

Practical consequence: the rollback plan is "move `T` further out", and it only
works while `T` is still in the future. Decide the timestamp with enough margin to
abort.

### Pre-flight, all before `T`

1. Every EL runs a binary that implements EIP-8312 — verify with
   `web3_clientVersion` on each, not on one.
2. Every EL's chain config carries the same `utxoFramesTime`. A node missing it
   treats EIP-8312 as unscheduled and will reject UTXO transactions its peers
   accept. The field is invisible to ForkId, so nothing at the peer layer catches
   a straggler.
3. `T` is comfortably after the current head's timestamp and after Hegotá.
   Scheduling it at or before an existing block's timestamp retroactively
   redefines that block.
4. Confirm the vault address is empty or, if not, that its storage is empty:
   activation preserves a pre-existing balance as inert surplus but assumes no
   storage.

### At the boundary

5. `eth_getCode` on `0x…8312` returns exactly **76 bytes**, and the account's
   nonce is 1.
6. The ring slot for the activation block, `1 + N % 8192`, is written — the
   all-zeros root of an empty block still counts and must be present.
7. The activation block's hash agrees on all three ELs, and the chain keeps
   advancing across it without a pause in block production.
8. Restart one EL (not the bootnode) and confirm its replay tail crosses the
   boundary with **no** `World State Root does not match`. This is the check that
   proves the history stays re-executable.

### End-to-end, after the boundary

9. Deposit, then spend it in a later block via a ring proof — the suite
   `utxo_itest.py` in this directory covers this and six other cases; run it
   against one EL with `--peers` naming the other two.
10. A sponsored spend (a sponsor pays, the spend keeps its full change).
11. A consolidation spend: several separately-owned UTXOs merged into one payout.
12. A batch-proof spend, which needs a chain older than `BATCH_SIZE` (8,192)
    blocks — roughly 14 hours at 6-second slots, so it is a follow-up rather
    than a boundary-day check.

## Post-upgrade verification checklist (every path)

Run all of it; a node that starts is not a node that works. Verify on the
node's **current** host RPC port (re-derive it after restart on dynamic-publish
enclaves — see Path 1 step 5).

1. `web3_clientVersion` on **every** EL shows the expected commit.
1b. If a wrapper is in play, the flags it adds still take effect — for the `ethrex`
   namespace, a bad-input call to one of its methods returns a handler error
   (`-32000`), **not** `-32601 Method not found`. A swap that clobbered the wrapper
   passes every other check on this list and fails only this one.
2. 3-EL consensus: same head number **and hash** on all ELs (cross-check a
   recent block's hash across el-1/2/3 — this also proves a partially-upgraded
   fleet still agrees, i.e. the change really is non-consensus).
3. **State preserved** (Paths 1/1b/2): a deep block's hash (e.g. block 1000) is
   **identical before and after** on every EL, and the startup log shows
   `Finished regenerating state` with **no** `World State Root does not match`.
4. Finality advancing (safe/finalized within normal distance of head).
5. Predeploys present (`eth_getCode` on `0x…8141`, `0x…8250`; `0x…8272` is
   intentionally empty-code) — Hegotá stack only.
6. A live frame transaction mines with status `0x1` (the submitter script in
   this directory), plus a regular EIP-1559 tx.
7. If the upgrade added/changed an RPC surface: the new method is **reachable**
   (a bad-input call returns a handler error like `-32000`, not `-32601 Method
   not found`) and the previously-served namespaces still respond.
8. Public endpoints respond through the reverse proxy / DNS names.
9. Restart policies (`docker update --restart unless-stopped`) still set on every
   container so a host reboot doesn't drop the devnet (in-place `docker restart`
   preserves them).
10. Rollback still possible: old image tag exists; for Paths 1/1b the wrapper
    can be reverted by restoring the real binary over it.

## Quick decision table

| Change | Path |
|---|---|
| Non-consensus change in the **binary** (RPC handler, mempool policy, P2P, builder selection, logging) | 1 — binary swap |
| Non-consensus change in the **container command** (add an RPC namespace, tune `--http.api` or a flag), state-preserving | 1b — wrapper |
| New EIP or gas rule, fork not yet active | 2 — swap pre-fork, activate later |
| Fixing rules of a fork already crossed / changing active behavior | 2-variant — new future fork |
| Re-genesis (wipe + re-run) | **FORBIDDEN** — never on this devnet |
| CL fork schedule / CL genesis params | escalate to the owner (would need forbidden re-genesis) |
| CL image only (same fork config) | 1-style CL swap, one node at a time |

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
4. Per EL: `docker cp /tmp/ethrex-new <ctr>:/usr/local/bin/ethrex` then
   `docker restart -t 20 <ctr>`.
5. **Host ports:** enclaves with `port_publisher` set have **deterministic**
   ports that survive restart. Enclaves that publish dynamically (older kurtosis)
   **remap the host port on every restart** — re-read it with
   `docker port <ctr> 8545` (or `kurtosis port print`) before verifying.
6. Verify (checklist below) before touching the next EL.
7. Rollback: keep the previous image (`docker tag ethrex:<current> ethrex:rollback-<sha>`)
   so you can `docker cp` the old binary back and restart.

**Durability:** the swapped binary lives in the writable layer — it survives
reboots (set `docker update --restart unless-stopped` on all containers) but a
container **recreate** reverts to the image. Retag the image
(`docker tag ethrex:hegota-<short-sha> ethrex:hegota`) so a future recreate uses
the new binary.

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

## Post-upgrade verification checklist (every path)

Run all of it; a node that starts is not a node that works. Verify on the
node's **current** host RPC port (re-derive it after restart on dynamic-publish
enclaves — see Path 1 step 5).

1. `web3_clientVersion` on **every** EL shows the expected commit.
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

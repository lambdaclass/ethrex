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

- **No** → the change is non-consensus. Path 1 (in-place swap).
- **Yes, but only for a fork the chain has NOT reached** → Path 2
  (deploy before the fork).
- **Yes, for rules the chain has ALREADY been running** → Path 3 (re-genesis),
  or the new-fork decoupling variant if history must survive.

Be paranoid with this test. Changes that *look* operational can be consensus:

- Gas accounting and refunds (a frame can observe the payer's balance
  mid-transaction — charging a different amount at APPROVE is
  consensus-visible even when the net end-of-tx balance is identical).
- Predeploy installs (they execute at a fork boundary and produce an account
  update in the state root).
- Anything touching `total_gas_limit`, intrinsic gas, opcode gas, journaling,
  or the BAL (EIP-7928) footprint.

Genuinely non-consensus: RPC handlers, mempool admission policy (execution
still validates in full), P2P, logging, docs, the payload *builder's* tx
selection (not its execution).

## Path 1 — In-place rolling swap (non-consensus changes)

Zero downtime, zero history loss. Swap one EL at a time and verify consensus
between steps.

1. Build the new image under a **unique immutable tag** (never reuse a mutable
   tag like `ethrex:hegota` on a shared build host — a concurrent build can
   hijack it): `make build-image TAG=hegota-<short-sha>`.
2. Tag the running image as a rollback:
   `docker tag ethrex:<current> ethrex:rollback-<current-sha>`.
3. Canary first: upgrade **el-2** (never the bootnode first), then el-3, then
   el-1.
4. `kurtosis service update --image ...` **drops all file mounts**
   (`/network-configs`, `/jwt`) — the EL will exit with
   `Failed to open genesis file` unless you pass them back:
   `--files "/network-configs:<artifact>,/jwt:jwt_file"` (re-upload the
   artifact with `kurtosis files upload` if you changed the genesis).
5. The EL datadir lives in the **container writable layer**, not a volume —
   `service update` destroys it. Sandwich every swap with
   `docker cp <ctr>:/data/ethrex ./bk-elN` before and a restore after, or the
   node will resync from genesis.
6. Host ports **remap** after `service update` — re-read them with
   `kurtosis port print` and update anything that references them.
7. Verify after each node (see the checklist below) before touching the next.

## Path 2 — Deploy before the fork (the mainnet-standard upgrade)

For consensus changes gated on a fork timestamp the chain has **not** reached.

1. Ensure every new rule is gated `fork >= NewFork` (predeploy installs must be
   idempotent and gated the same way — they re-run on every block during
   re-execution, which is what makes replay safe).
2. Schedule the fork **in the future** in the genesis/chain config. Setting a
   fork time at or before existing block timestamps retroactively redefines
   history → guaranteed state-root mismatch.
3. Swap all ELs (Path 1 mechanics) **while the chain is still pre-fork**. The
   new binary re-executes pre-fork blocks under pre-fork rules — identical
   results — and the fork later activates under the new binary.
4. If the fork also needs CL awareness (new engine-API version, new payload
   fields), confirm the CL release supports it *before* scheduling.

### Variant: new-fork decoupling (history-preserving change to active behavior)

If the chain already crossed fork F and you must change behavior that F
introduced, do **not** redefine F. Gate the change on a new config knob /
successor fork F′ set in the future (precedent: the `postHegotaTime` field
that deferred the NONCE_MANAGER install on an already-post-Hegotá chain).
This preserves history at the cost of a non-canonical fork layout — acceptable
for keeping a long-lived chain alive, wrong for a devnet whose purpose is
testing the canonical layout. Fresh chains must keep working with the knob
unset (fall back to F).

## Path 3 — Re-genesis (canonical redefinition)

For consensus changes to an already-active fork on a devnet where history is
expendable. This wipes the chain: all balances, contracts, and history are
gone, and users must re-claim from the faucet — **announce that explicitly**
(always state whether users need to do anything, even when the answer is no).

1. Build + verify the new image (unique tag; check
   `docker run --rm --entrypoint ./ethrex <image> --version` shows the
   expected commit).
2. Keep the old image tagged as a rollback.
3. `kurtosis enclave rm -f <enclave>` then
   `kurtosis run --enclave <enclave> <ethereum-package> --args-file <config>`
   with the **pinned** ethereum-package revision (`make checkout-ethereum-package`).
4. With `port_publisher.public_port_start` set, EL/CL host ports are
   **deterministic** across fresh runs (`start + participant*7 + offset`), so
   reverse proxies and the faucet keep working. **Additional services (Dora)
   are NOT deterministic** — front them through a stable local forward (a
   `socat` systemd unit) and update only that unit's target port after each
   re-genesis, never the proxy config.
5. Kurtosis leaves every container with `RestartPolicy=no` and has no
   `enclave start` — a host reboot would kill the devnet permanently. After
   every deploy: `docker update --restart unless-stopped` on all enclave
   containers (and the faucet).
6. CL fork-epoch changes (the `network_params` epochs) are part of the CL
   genesis state — they can only change via re-genesis. The CL release must
   know every scheduled fork name or it will reject the genesis outright.

## Post-upgrade verification checklist (every path)

Run all of it; a node that starts is not a node that works.

1. `web3_clientVersion` on **every** EL shows the expected commit.
2. 3-EL consensus: same head number **and hash** on all ELs.
3. Finality advancing (safe/finalized within normal distance of head).
4. Predeploys present (`eth_getCode` on `0x…8141`, `0x…8250`; `0x…8272` is
   intentionally empty-code).
5. A live frame transaction mines with status `0x1` (the submitter script in
   this directory), plus a regular EIP-1559 tx.
6. Public endpoints respond through the reverse proxy / DNS names.
7. Restart policies applied (step 5 of Path 3).
8. Rollback still possible: old image tag exists; for Path 1, datadir backups
   exist.

## Quick decision table

| Change | Path |
|---|---|
| RPC / mempool policy / P2P / builder selection / docs | 1 — rolling swap |
| New EIP or gas rule, fork not yet active | 2 — swap pre-fork, activate later |
| Fixing rules of a fork already crossed (devnet) | 3 — re-genesis |
| Changing active behavior, history must survive | 2-variant — new future fork |
| CL fork schedule / CL genesis params | 3 — re-genesis |
| CL image only (same fork config) | 1-style CL swap, one node at a time |

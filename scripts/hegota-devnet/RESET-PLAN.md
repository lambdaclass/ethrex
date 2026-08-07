# Hegotá devnet reset and re-genesis — plan

Deliberate exception to the re-genesis prohibition in `UPGRADE-GUIDE.md`, approved
2026-08-06. Same host, same infrastructure, same chain id: the chain is wiped and
restarted, not replaced.

Two things drive it. FOCIL cannot be reached by upgrade (see [Why an upgrade is
impossible](#why-an-upgrade-is-impossible)), and management asked for ten
prefunded rich wallets on non-public keys.

## Why an upgrade is impossible

The chain has `HEZE_FORK_EPOCH: 2` in its consensus config, but every client
running since genesis was built from a branch whose `ForkName` stops at `Gloas`.
So the chain ran past its own fork boundary under the wrong rules, for ~3450
epochs.

That is not cosmetic. Heze changes the SSZ layout: `ExecutionPayloadBid` gains

```rust
#[superstruct(only(Heze))]
pub inclusion_list_bits: BitVector<E::InclusionListCommitteeSize>,
```

so a Heze block body differs structurally from a Gloas one. Every block from
epoch 2 onward carries a Gloas-shaped body while any heze-aware client expects a
Heze-shaped one. No binary swap, datadir preservation or rolling restart fixes
that; the history itself is the incompatibility.

Note the failure mode for next time: **scheduling a fork that no client
implements is silently corrupting.** The config looked correct and the chain ran
fine, because the only clients present agreed to ignore it.

## The gate that decides the shape of the reset

`ethpandaops/lighthouse:focil` is unproven on this network config. It has the
right fork variants, `JsonPayloadAttributesV4`/`V5` carrying `slot_number` and
`target_gas_limit`, and DB schema 29 matching the current build. What is untested
is the rest: the BPO/blob schedule and PeerDAS at `fulu: 0`. The branch sits ~104
commits behind `unstable`.

**Run a throwaway enclave with that image before touching the live one.** The
result selects the target:

| Result | Target |
|---|---|
| It drives the chain | **Config A**, below. Full stack including FOCIL. |
| It does not | **Config B**. Reset for the rich wallets only, FOCIL deferred. |

Do not skip this. A reset that lands on a broken consensus client leaves no
devnet at all, which is strictly worse than the current position.

### Config A — frame transactions plus FOCIL

- `cl_image: ethpandaops/lighthouse:focil` on all three participants.
- `el_image: ethrex:local` built from `hegota-devnet` HEAD.
- Fork schedule unchanged: `fulu: 0`, `gloas: 1`, `heze: 2`.

Everything is consistent from block zero, which is the whole point of the reset.

### Config B — frame transactions only

If the FOCIL client fails, **do not simply keep `heze_fork_epoch: 2` with a
devnet-7 client.** That reproduces exactly the corruption described above and
burns the next reset too.

Config B needs one of:

- the pre-FOCIL EL image (`f3902e177` or equivalent), keeping today's behaviour
  and gaining only the rich wallets; or
- reintroducing a separate FOCIL activation timestamp so a HEAD EL can run with
  FOCIL dormant. This was implemented and then reverted (`5cd982b2c`) because
  `hegota_time` already *is* the heze activation. Reintroducing it is a
  deliberate, documented divergence, not a bug fix.

Config B is a real decision, not a fallback to improvise at 2am. Settle which
variant before starting.

## Why a HEAD EL cannot be deployed to the current chain

Worth stating so nobody tries the cheaper thing first. The engine version guards
resolve through `is_hegota_activated`, and Hegotá activated 2026-07-29. A HEAD EL
therefore rejects `newPayloadV5` and `forkchoiceUpdatedV4` immediately, and the
running Lighthouse speaks nothing else. It halts the chain on contact. Correct
behaviour, and the reason the reset is the only route.

## Fix the persistence model while we are here

Today neither the EL nor the CL persists chain data to a volume. Only `/jwt` and
`/network-configs` are mounted, so `/data/ethrex/execution-data` and
`/data/lighthouse/beacon-data` (2.3 GB) live in the container writable layer.
Recreating a container destroys that node's chain.

That single fact is why `UPGRADE-GUIDE.md` Path 1 is a *binary swap into a
running container* rather than an ordinary image change, and it is what would
have made a naive FOCIL client swap unrecoverable across all three nodes at once.

The package already supports the fix. `persistent: true` at the top level of the
fixture makes both launchers mount their data directories instead:

```python
if persistent:
    files[EXECUTION_DATA_DIRPATH_ON_CLIENT_CONTAINER] = el_shared.get_persistent_data_directory(...)
```

which resolves to `Directory(persistent_key="data-<service_name>", size=...)`, a
Kurtosis-managed volume keyed to the service. It survives container recreation
inside the enclave, so a future client upgrade becomes a normal image bump rather
than a hand-staged binary. `kurtosis enclave rm` still removes it, which is
correct: that is the intentional teardown, not an upgrade.

**Validate it in the same throwaway enclave as the client gate.** The parameter
is marked *"WIP and slowly being rolled out across services"* upstream, and the
README frames it around Kubernetes storage, so do not adopt it blind during a
production reset. Confirm on the scratch enclave that both data directories
appear as volumes in `docker inspect` and that a container recreation keeps the
chain. Also set `el_volume_size` / `cl_volume_size` deliberately; the default is
`0`.

If it validates, the payoff is that Path 1 can eventually be retired. If it does
not, the reset proceeds without it and nothing is lost.

### The faucet is ours, and simpler

The faucet is a plain `docker run`, not a Kurtosis service, so it needs no
package support. Mount a host directory over `/app`'s static files, or at minimum
bind-mount `page.html` and `eips.html`, so updating the landing page or the guide
stops requiring an image rebuild. That is what forced today's `docker cp`, which
left the running container ahead of `hegota-faucet:page-v3`.

Keep `faucet.py` in the image. Code should ship as a build, not as a file someone
edited on the host.

## Preserve across the reset

| Item | Value |
|---|---|
| Chain id | `3151908` (`0x301824`) |
| Fork schedule | `fulu: 0`, `gloas: 1`, `heze: 2`, 6s slots |
| EIP-8282 predeploys | the two `additional_preloaded_contracts` entries, unchanged |
| ethrex knobs | `--syncmode full`, `--mempool.max-verify-gas 500000` |
| Public hostnames | `rpc1/rpc2/rpc3/dora/faucet.hegota.ethrex.xyz` |
| Port layout | EL 32000+, CL 31000+, services 31500+ |

Caddy needs no change if the port layout holds. Confirm it does; Kurtosis
reassigns ports on re-genesis, which is why Dora already goes through a fixed
socat forward (`hegota-dora-forward.service`) rather than a direct port.

## Set at genesis, not after

**Ten rich wallets** go in `prefunded_accounts` in
`fixtures/networks/hegota-devnet.yaml`. Balances are genesis state, so they
cannot be patched in afterwards without changing the state root.

Addresses are in the launch canvas; keys are in `richwallets.tsv`, kept out of
this file deliberately.

**The faucet account** is generated separately and does **not** go in the shared
canvas. It also goes in `prefunded_accounts`, funded ~100000 ETH. The current
faucet key (`0x2063…66AE`) and the three interim accounts from `NOTES-LOCAL.md`
are compromised: they were shared over chat. Do not carry any of them forward.

Kurtosis' own defaults prefund ~20 accounts from the well-known mnemonic. Those
stay reachable by anyone, which is how the old faucet's nonce was clobbered by
ordinary test traffic. The rich wallets exist precisely so that funded accounts
are not public ones.

## Patched after genesis, before its timestamp

`utxoFramesTime` is not emitted by `ethereum-genesis-generator`, so EIP-8312 is
scheduled the same way it was last time: write the field into every EL's
`/network-configs/genesis.json` once the enclave is up, then restart those ELs.

Set it to a timestamp comfortably in the future and finish patching every node
before it arrives. A node missing the field treats EIP-8312 as unscheduled and
rejects UTXO transactions its peers accept, and the field is invisible to ForkId
so nothing at the peer layer catches a straggler.

Do not anchor a `sed` on a fork name: the execution config calls the Hegotá fork
`bogotaTime`, so matching on `hezeTime` silently matches nothing while reporting
success. Patch the JSON on the host, `docker cp` in and out; the EL containers
have no Python.

## Sequence

1. **Gate.** One throwaway enclave answers both open questions: does
   `lighthouse:focil` drive this config, and does `persistent: true` give real
   volumes on the Docker backend. Pick Config A or B, and decide whether
   persistence ships with this reset.
2. **Snapshot.** ~7.5 GB (3× 192 MB EL, 3× 2.3 GB CL) against ~898 GB free. Keep
   it until the new chain has been healthy for a day.
3. **Build** `ethrex:local` from `hegota-devnet` HEAD on the host.
4. **Rebuild the faucet image** as `hegota-faucet:page-v4`. The running container
   was updated in place with `docker cp`, so the `/eips` guide exists only in the
   container, not the image. Recreating it from `page-v3` loses the guide.
5. **Edit the fixture**: `prefunded_accounts` (ten wallets plus the faucet
   account), `cl_image` if Config A, and `persistent: true` plus
   `el_volume_size` / `cl_volume_size` if the gate cleared it.
6. **Tear down and relaunch** the enclave.
7. **Patch `utxoFramesTime`** into all three ELs, restart them, before `T`.
8. **Recreate the faucet** from `page-v4` with the new `PRIVATE_KEY` and
   `RPC_URL`, and with the static files bind-mounted so the next page change
   needs no rebuild.
9. **Verify**, below.
10. **Announce**, and rotate anything that was shared during bring-up.

## Verification

- All three ELs report the same head, and `eth_syncing` is false.
- `debug_chainConfig` on each EL shows the same `hegotaTime`, `utxoFramesTime`
  and chain id. A per-node diff here is the failure this whole document is about.
- Under Config A: `engine_exchangeCapabilities` advertises
  `engine_getInclusionListV1`, `engine_forkchoiceUpdatedV5` and
  `engine_newPayloadV6`, and `/eth/v1/config/spec` on the CL now **reports
  `HEZE_FORK_EPOCH`**. Its absence is what the old chain got wrong.
- The vault predeploy at `0x…8312` has code once `utxoFramesTime` passes.
- Each rich wallet shows its genesis balance.
- Faucet: `/healthz` reports the new address, a claim succeeds, and `/eips`
  serves the guide.
- A frame transaction lands, via `scripts/hegota-devnet/frametx_submit.py`.
- If persistence shipped: `docker inspect` shows both data directories as
  volumes, and recreating one EL container leaves its chain intact.

## Rollback

Until step 6 there is nothing to roll back. After it, the old chain exists only
as the step 2 snapshot; restoring means recreating containers against those
datadirs, and the public endpoints will have moved on. Treat step 6 as the point
of no return and confirm the gate result before crossing it.

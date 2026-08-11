# Hegotá Devnet — Genesis Requirements

What a fresh genesis for this devnet must contain, and why. Every item here is
load-bearing: omitting any of the first two sections produces a chain that either
never starts or stops producing blocks at a fork boundary.

The companion documents are `hegota-devnet.md` (what the branch is composed of and
where it diverges from the draft EIPs), `scripts/hegota-devnet/USER-GUIDE.md` (how to
use the devnet) and `scripts/hegota-devnet/UPGRADE-GUIDE.md` (how to change a
*running* devnet without a re-genesis).

## Fork schedule

`fixtures/networks/hegota-devnet.yaml` schedules Osaka from genesis, Amsterdam at
epoch 1 and Hegotá at epoch 2.

**Amsterdam must be scheduled.** Hegotá is defined on top of Amsterdam. levm
resolves Amsterdam's EVM rules from the fork *ordinal* (`fork >= Fork::Amsterdam`,
which `Fork::Hegota` satisfies), while the header schema, the block access list and
the Engine API surface all key on the `amsterdamTime` *field*
(`ChainConfig::is_amsterdam_activated`). A genesis that sets `hegotaTime` and
leaves `amsterdamTime` unset therefore runs Amsterdam EVM rules under a
pre-Amsterdam header schema. `validate_prague_header_fields` is the consensus-level
instance of that split; mempool admission is the instance PR #7053 fixes.

Scheduling Amsterdam also selects the Engine API version. Once it is active ethrex
rejects `engine_forkchoiceUpdatedV3` payload attributes outright, so the consensus
client **must** drive `engine_forkchoiceUpdatedV4` and supply the EIP-7843
`slotNumber` that an Amsterdam header requires. `ethpandaops/lighthouse:glamsterdam-devnet-7`
does; a stable Lighthouse release generally does not.

**Amsterdam must not be at genesis.** With `gloas_fork_epoch: 0` the beacon genesis
state is a Gloas state, and Lighthouse rejects the one the genesis generator writes:

```
CRIT Failed to start beacon node
     reason: "Built-in genesis state SSZ bytes are invalid: OffsetsAreDecreasing(0)"
```

Scheduling Gloas at epoch 1 or later keeps the genesis state Fulu and lets the
consensus client fork itself into Gloas. The sibling `bal-devnet-*` configs use the
same arrangement.

## Required predeploys

**EIP-8282 builder execution requests** — `BUILDER_DEPOSIT_CONTRACT_ADDRESS`
(`0x0000BFF46984E3725691FA540A8C7589300D8282`) and `BUILDER_EXIT_CONTRACT_ADDRESS`
(`0x000064D678505AD48F8CCB093BC65613800E8282`). EIP-8282 is part of Glamsterdam
(EIP-7773), and it specifies that if either address has no code once the EIP is
active then **every block from activation onward is invalid** — the same rule
EIP-7002 and EIP-7251 apply to their own predeploys. ethrex enforces it via
`AMSTERDAM_REQUEST_PREDEPLOYS` in `crates/vm/system_contracts.rs`; the symptom of a
missing predeploy is that `getPayload` fails on every slot from the Amsterdam
boundary on, the consensus client reports `Unknown payload`, and the chain freezes
on the last pre-Amsterdam block.

`ethereum-genesis-generator` deploys these from **6.1.4** onward, which is what
`ETHEREUM_PACKAGE_REVISION` pins, so the code no longer has to come from the config.
The config still preloads both through `network_params.additional_preloaded_contracts`
for one reason: 6.1.4 sets storage slot 0 to `EXCESS_INHIBITOR` (`2**256-1`) on the
*exit* predeploy but not on the *deposit* one, and EIP-8282 arms both at deployment so
that no request can be enqueued before the first end-of-block system call. A preloaded
runtime image never runs the init code that would otherwise arm it. The generator
applies its system contracts before the additional contracts, so the preload wins and
supplies the inhibitor the generator omits.

**Keep that block until the generator arms the deposit predeploy's inhibitor**, then
delete it so the predeploys come from one place.

**Hegotá predeploys** — `0x…8141` (`EXPIRY_VERIFIER`) and `0x…8250`
(`NONCE_MANAGER`) are installed by ethrex at the fork and need no genesis entry.
`0x…8272` (`RECENT_ROOT_ADDRESS`) is intentionally empty-code; its 64-byte write is
handled natively.

## ethrex-only chain-config fields

The genesis generator does not emit these, so they are absent from a fresh genesis
and must be patched into `/network-configs/genesis.json` if wanted.

- `derivedSlotTime` — derives the beacon slot from the block timestamp for a
  consensus client that does not supply the EIP-7843 slot. Unnecessary on a chain
  that schedules Amsterdam with an EIP-7843-capable client: the slot arrives over
  `engine_forkchoiceUpdatedV4`, lands in the header, and
  `ChainConfig::effective_slot_number` returns it verbatim without consulting the
  derivation. Leave unset. Its two companion fields, `genesisTimestamp` and
  `secondsPerSlot`, exist only to feed this derivation.
- `payerTxparamTime` — activates the resolved-payer `TXPARAM(0x11)` index. Unset
  means the index keeps its `InvalidOpcode` halt, so the extension is silently off
  on a fresh chain.

## Host requirements

`port_publisher` blocks must sit below the host's ephemeral port floor
(`/proc/sys/net/ipv4/ip_local_port_range`, `32768` by default). Kurtosis assigns
dynamic host ports to unpublished services out of that range, so a *fixed* publish
inside it can lose the race against one of them:

```
failed to bind host port 0.0.0.0:33008/tcp: address already in use
```

The `el` block at `32000` is safe. The `cl` block at `33000` and the
`additional_services` block at `34000` are both inside the default ephemeral range;
either move them below `32768` or raise the host's floor above the highest
published port.

## Explorer

`dora_params.image` must be `ghcr.io/lambdaclass/dora:frame-tx-view`, which decodes
the type-`0x06` envelope and shows per-frame receipts. Stock `ethpandaops/dora`
renders frame transactions as opaque blobs.

## Verification after deployment

Run all of it. A chain that starts is not a chain that works, and the failure modes
above are all silent until a specific fork boundary.

1. The startup banner lists Osaka, Amsterdam and Hegotá, each with a non-zero
   timestamp except Osaka.
2. `debug_chainConfig` on every EL reports the same `amsterdamTime` and
   `hegotaTime`, and the genesis hash matches
   `/network-configs/deposit_contract_block_hash.txt` (the block hash the consensus
   genesis embeds).
3. The chain crosses the Amsterdam boundary: the last pre-Amsterdam block has
   neither `slotNumber` nor `blockAccessListHash`, and the first Amsterdam block has
   both, with `slotNumber` supplied by the consensus client rather than derived.
4. `requestsHash` on an Amsterdam block is the empty-keccak
   `0xe3b0c44298fc1c14…` while the EIP-8282 queues are empty — proof the predeploys
   are being system-called and returning cleanly rather than erroring.
5. The chain crosses the Hegotá boundary, and `eth_getCode` on `0x…8141` and
   `0x…8250` returns non-empty.
6. Every EL agrees on the head number **and hash**.
7. A frame transaction mines with status `0x1` and per-frame `frameReceipts`, plus a
   regular EIP-1559 transaction.

Checks 8 to 12 apply to the Hegotá **testnet** and not to a local devnet: they cover
external reachability, FOCIL, the EIP-8272 predeploy and the gated deposit contract,
none of which a single-host devnet exercises. Ports and firewall surface are in
`docs/hegota-testnet-joining.md`.

8. Every EL advertises `<PUBLIC_IP>` in its `admin_nodeInfo` enode, and an `ethrex
   --bootnodes` node started **on a different host** completes discovery and reaches
   the head. Reachability cannot be verified from the host itself: a rule that opens
   only TCP, or a node advertising a container-internal address, both look healthy
   from inside and are unreachable from outside.
9. `engine_getInclusionListV1` returns a non-empty list on a slot with a full mempool,
   an inclusion list delivered on `engine_newPayloadV6` is honoured by the builder,
   and an omitted frame transaction is excused rather than yielding
   `inclusionListSatisfied: false`. Confirm the third one against a frame transaction
   that is genuinely ineligible under EIP-8369 Profile 2, not merely absent: an
   eligible omission is *supposed* to report unsatisfied, so a test that cannot tell
   the two apart proves nothing.
10. `eth_getCode` on `0x…8272` returns exactly the 144-byte `RECENT_ROOT_CODE`, and a
    plain EOA transaction to it with 64 bytes of calldata writes an entry that a
    subsequent frame transaction successfully references. Record the observed gas: it
    is emergent from the EVM rather than a constant, so it is a measurement, not an
    assertion.
11. A deposit from an address holding no gater token reverts with "Not enough tokens",
    and the same deposit succeeds after `gating-cli mint`. Both halves are required:
    only the second proves the gate is not simply broken for everyone.
12. That deposit produces a standard `DepositEvent` that appears in the block's
    `requestsHash`, and the consensus client activates the validator. `eth_getLogs` on
    the deposit address shows `DEPOSIT_TOPIC`
    `0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5` and **no
    additional event** — an extra event would mean the gated contract is not
    transparent to the execution layer after all.

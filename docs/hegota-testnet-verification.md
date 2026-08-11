# Hegotá testnet — genesis requirements and verification

What a genesis for this testnet must contain, and how to prove a deployment works.
Every item in the first two sections is load-bearing: omitting any of them produces a
chain that either never starts or stops producing blocks at a fork boundary.

Companion documents: `scripts/hegota-testnet/INSTALL.md` (the install runbook, which
sends you here for section "Verification"), `docs/hegota-testnet-joining.md` (what a
joiner consumes, and the firewall surface),
`docs/hegota-testnet-upgrading.md` (changing a running chain),
`docs/hegota-testnet-divergences.md` (where the implementation departs from the drafts).

## Fork schedule

`fixtures/networks/hegota-testnet.yaml` schedules Osaka from genesis, Amsterdam at
epoch 1 and Hegotá at epoch 2. On the consensus side the same boundaries are
`fulu_fork_epoch: 0`, `gloas_fork_epoch: 1`, `heze_fork_epoch: 2`.

**Amsterdam must be scheduled.** Hegotá is defined on top of Amsterdam. levm resolves
Amsterdam's EVM rules from the fork *ordinal* (`fork >= Fork::Amsterdam`, which
`Fork::Hegota` satisfies), while the header schema, the block access list and the
Engine API surface all key on the `amsterdamTime` *field*
(`ChainConfig::is_amsterdam_activated`). A genesis that sets `hegotaTime` and leaves
`amsterdamTime` unset therefore runs Amsterdam EVM rules under a pre-Amsterdam header
schema. `ChainConfig::validate_fork_schedule` rejects that combination at genesis load,
because EIP-8081 (*Hardfork Meta - Hegotá*) declares `requires: [7723, 7773]` and
EIP-7773 is Amsterdam's own meta.

Scheduling Amsterdam also selects the Engine API version. Once it is active ethrex
rejects `engine_forkchoiceUpdatedV3` payload attributes outright, so the consensus
client **must** drive `engine_forkchoiceUpdatedV4` and supply the EIP-7843 `slotNumber`
that an Amsterdam header requires.

**Amsterdam must not be at genesis.** With `gloas_fork_epoch: 0` the beacon genesis
state is a Gloas state, and Lighthouse rejects the one the genesis generator writes:

```
CRIT Failed to start beacon node
     reason: "Built-in genesis state SSZ bytes are invalid: OffsetsAreDecreasing(0)"
```

Scheduling Gloas at epoch 1 or later keeps the genesis state Fulu and lets the
consensus client fork itself into Gloas.

**Scheduling Gloas also sets the gas limit to 200,000,000.** The pinned
`ethereum-package` defaults both `genesis_gaslimit` and `gas_limit` to that value
whenever `gloas_fork_epoch` is present, and the testnet config does not override
either. Any constant justified by arithmetic against a 60,000,000 block is wrong here
by more than 3×; check the effective limit before reasoning from one.

## Required predeploys

**EIP-8282 builder execution requests** — `BUILDER_DEPOSIT_CONTRACT_ADDRESS`
(`0x0000BFF46984E3725691FA540A8C7589300D8282`) and `BUILDER_EXIT_CONTRACT_ADDRESS`
(`0x000064D678505AD48F8CCB093BC65613800E8282`). EIP-8282 is part of Glamsterdam
(EIP-7773), and it specifies that if either address has no code once the EIP is active
then **every block from activation onward is invalid** — the same rule EIP-7002 and
EIP-7251 apply to their own predeploys. ethrex enforces it via
`AMSTERDAM_REQUEST_PREDEPLOYS` in `crates/vm/system_contracts.rs`; the symptom of a
missing predeploy is that `getPayload` fails on every slot from the Amsterdam boundary
on, the consensus client reports `Unknown payload`, and the chain freezes on the last
pre-Amsterdam block.

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

**Hegotá predeploys** — `0x…8141` (`EXPIRY_VERIFIER`), `0x…8250` (`NONCE_MANAGER`) and
`0x…8272` (`RECENT_ROOT_ADDRESS`) are installed by ethrex at the fork and need no
genesis entry.

`0x…8272` must be empty-code **at genesis** and not merely absent from it. EIP-8272
"Activation" requires the address to hold empty code and empty storage in the parent
state of the first post-fork payload, and declares the payload invalid otherwise;
ethrex then sets its code to `RECENT_ROOT_CODE` at the boundary. Post-fork the address
therefore holds code, which is what check 10 asserts — an address still empty after the
boundary means the activation transition did not run.

## ethrex-only chain-config fields

The genesis generator does not emit these, so they are absent from a fresh genesis and
must be patched into `/network-configs/genesis.json` if wanted.

- `derivedSlotTime` — derives the beacon slot from the block timestamp for a consensus
  client that does not supply the EIP-7843 slot. Unnecessary on a chain that schedules
  Amsterdam with an EIP-7843-capable client: the slot arrives over
  `engine_forkchoiceUpdatedV4`, lands in the header, and
  `ChainConfig::effective_slot_number` returns it verbatim without consulting the
  derivation. Leave unset. Its two companion fields, `genesisTimestamp` and
  `secondsPerSlot`, exist only to feed this derivation.
- `payerTxparamTime` — activates the resolved-payer `TXPARAM(0x11)` index. Unset means
  the index keeps its `InvalidOpcode` halt, so the extension is silently off on a fresh
  chain.

## Host requirements

`port_publisher` blocks must sit below the host's ephemeral port floor
(`/proc/sys/net/ipv4/ip_local_port_range`, `32768` by default). Kurtosis assigns dynamic
host ports to unpublished services out of that range, so a *fixed* publish inside it can
lose the race against one of them:

```
failed to bind host port 0.0.0.0:33008/tcp: address already in use
```

The testnet's `el` block at `32000`, `cl` block at `31000` and `additional_services`
block at `31500` are all below the default floor.

## Verification after deployment

Run all of it. A chain that starts is not a chain that works, and the failure modes
above are all silent until a specific fork boundary.

1. The startup banner lists Osaka, Amsterdam and Hegotá, each with a non-zero timestamp
   except Osaka.
2. `debug_chainConfig` on every EL reports the same `amsterdamTime` and `hegotaTime`,
   and the genesis hash matches `/network-configs/deposit_contract_block_hash.txt` (the
   block hash the consensus genesis embeds).
3. The chain crosses the Amsterdam boundary: the last pre-Amsterdam block has neither
   `slotNumber` nor `blockAccessListHash`, and the first Amsterdam block has both, with
   `slotNumber` supplied by the consensus client rather than derived.
4. `requestsHash` on an Amsterdam block is the empty-keccak `0xe3b0c44298fc1c14…` while
   the EIP-8282 queues are empty — proof the predeploys are being system-called and
   returning cleanly rather than erroring.
5. The chain crosses the Hegotá boundary, and `eth_getCode` on `0x…8141` and `0x…8250`
   returns non-empty.
6. Every EL agrees on the head number **and hash**.
7. A frame transaction mines with status `0x1` and per-frame `frameReceipts`, plus a
   regular EIP-1559 transaction.
8. Every EL advertises `<PUBLIC_IP>` in its `admin_nodeInfo` enode, and an `ethrex
   --bootnodes` node started **on a different host** completes discovery and reaches the
   head. Reachability cannot be verified from the host itself: a rule that opens only
   TCP, or a node advertising a container-internal address, both look healthy from
   inside and are unreachable from outside. An execution client alone is not a
   sufficient joiner — with no consensus client driving forkchoice it peers and then
   sits at genesis, logging `No messages from the consensus layer`. Start the joiner
   from the *published bundle* rather than from this repository, so the same run proves
   the bundle is complete.
9. `engine_getInclusionListV1` returns a non-empty list on a slot with a full mempool,
   an inclusion list delivered on `engine_newPayloadV6` is honoured by the builder, and
   an omitted frame transaction is excused rather than yielding
   `inclusionListSatisfied: false`. Confirm the third one against a frame transaction
   that is genuinely ineligible under EIP-8369 Profile 2, not merely absent: an eligible
   omission is *supposed* to report unsatisfied, so a test that cannot tell the two
   apart proves nothing.
10. `eth_getCode` on `0x…8272` returns exactly the 144-byte `RECENT_ROOT_CODE`, and a
    plain EOA transaction to it with 64 bytes of calldata writes an entry that a
    subsequent frame transaction successfully references. Record the observed gas: it is
    emergent from the EVM rather than a constant, so it is a measurement, not an
    assertion. Both rejection paths are part of the check — EIP-8272 requires a call to
    revert unless calldata is exactly 64 bytes *and* call value is zero.
11. A deposit from an address holding no gater token reverts with "Not enough tokens",
    and the same deposit succeeds after `gating-cli mint`. Both halves are required:
    only the second proves the gate is not simply broken for everyone.
12. That deposit produces a standard `DepositEvent` that appears in the block's
    `requestsHash`, and the consensus client activates the validator. `eth_getLogs` on
    the deposit address shows `DEPOSIT_TOPIC`
    `0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5` and **no
    additional event** — an extra event would mean the gated contract is not transparent
    to the execution layer after all.

Checks 7 and 9 to 12 need the frame-transaction submitters in `scripts/hegota-testnet/`
and the `pk910/gated-deposit-contract-cli` container; the rest need only `curl` and
`jq`.

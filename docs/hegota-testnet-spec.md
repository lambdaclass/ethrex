# hegota-testnet spec

:::info
:mega: **Live since 2026-09-03 17:18:26 UTC.** Chain ID `8141`, genesis hash
`0x7ca0f7358d127dc4a68983050eb88837a5f384225254d1b009fa87fbcd0f2332`. This is the third genesis
of the network: the previous chain ended at block 313,106 and was replaced, so a node that
followed it must wipe its database and resync from the new bundle.
:::

:::info
:checkered_flag: **Scope.** A public test network for the transaction-layer EIPs a shielded pool
needs from the base layer, run together on top of Glamsterdam: [EIP-8141 Frame
Transactions](https://eips.ethereum.org/EIPS/eip-8141), [EIP-8250 Keyed
Nonces](https://eips.ethereum.org/EIPS/eip-8250), [EIP-8272 Recent
Roots](https://eips.ethereum.org/EIPS/eip-8272) and [EIP-7805
FOCIL](https://eips.ethereum.org/EIPS/eip-7805), with
[EIP-8369](https://github.com/ethereum/EIPs/pull/12110) deciding which frame transactions an
inclusion list may force. All of them activate at one fork, **Hegotá** (`heze` on the
consensus layer, `bogotaTime` in the execution genesis). Anyone can sync, peer and
transact; only validator entry is permissioned.
:::

:::warning
:pushpin: **Each EIP is implemented at a pinned commit, not at the draft's head.** These
drafts move weekly. What runs here is the text at the commits in the table below, and a spec
change reaches the chain only through a relaunch. `git show <pin>:EIPS/eip-<n>.md` in
`ethereum/EIPs` is the exact text; the "Changed upstream since the pins" section lists what
has moved and is *not* on this chain.
:::

## EIP List for hegota-testnet

| EIP | Title | Pinned commit | Status |
|--------|-----|-----|:-----:|
| [EIP-8141](https://eips.ethereum.org/EIPS/eip-8141) | Frame Transaction | [`7d1c8bfb94`](https://github.com/ethereum/EIPs/blob/7d1c8bfb94/EIPS/eip-8141.md) (2026-08-24) | :new: |
| [EIP-8250](https://eips.ethereum.org/EIPS/eip-8250) | Keyed Nonces | [`e5cf246ff1`](https://github.com/ethereum/EIPs/blob/e5cf246ff1/EIPS/eip-8250.md) (2026-08-31) | :new: |
| [EIP-8272](https://eips.ethereum.org/EIPS/eip-8272) | Recent Roots | [`0231fb05f5`](https://github.com/ethereum/EIPs/blob/0231fb05f5/EIPS/eip-8272.md) (2026-08-31) | :new: |
| [EIP-7805](https://eips.ethereum.org/EIPS/eip-7805) | Fork-choice enforced Inclusion Lists (FOCIL) | [`9a345f96c2`](https://github.com/ethereum/EIPs/blob/9a345f96c2/EIPS/eip-7805.md) | :new: |
| [EIP-8369](https://github.com/ethereum/EIPs/pull/12110) | VOPS Profiles for FOCIL Eligibility | [`33724bd7da`](https://github.com/soispoke/EIPs/blob/33724bd7da/EIPS/eip-8369.md) on the PR branch, unmerged | :new: |

Each pin is the last commit that touched that EIP's file. EIP-8369 is in the set because
EIP-7805 enforcement over frame transactions is undefined without an eligibility rule, and
EIP-8369 is that rule; it has no activation switch of its own.

**Inherited, not chosen:** the fork sits on Glamsterdam (Gloas / Amsterdam), and every EIP in
that meta is live, including EIP-7928 block-level access lists, EIP-8037 state-creation gas
and EIP-8282 builder execution requests. A client without Glamsterdam cannot follow this chain
at any height.

**Not on this chain:** EIP-7906 (deleted from the branch) and EIP-8312 (present in the binary,
inert because `utxoFramesTime` is unset). And do not implement the fork from
[EIP-8081](https://eips.ethereum.org/EIPS/eip-8081)'s meta, which lists EIP-7805 alone under the
name Hegotá: a client built from it rejects every frame transaction on the chain.

## Network

| | |
|---|---|
| Chain ID / network ID | `8141` |
| Genesis | `1788455906` (2026-09-03 17:18:26 UTC), hash `0x7ca0f735…cd0f2332` |
| Slot / epoch | 6 s, 32 slots |
| Fork schedule | Fulu / Osaka at epoch 0 · Gloas / Amsterdam at epoch 1 (genesis + 192 s) · Heze / Hegotá at epoch 2 (genesis + 384 s) |
| Block gas limit | 200,000,000 |
| Validators | 96 at genesis (3 nodes × 32), operated by the ethrex team. Entry is permissioned: the deposit contract keeps the mainnet address but burns one access token per deposit; top-ups need none |
| Data availability | PeerDAS, 128 custody groups; all three operator beacon nodes are supernodes, which is what lets a default client range-sync on a network this small |
| Execution client | ethrex, branch [`hegota-testnet`](https://github.com/lambdaclass/ethrex/tree/hegota-testnet); the nodes run `31b532266` (`web3_clientVersion` reports it) |
| Consensus client | Lighthouse `ethpandaops/lighthouse:focil`, `v8.1.3-52e5197`. A stock Lighthouse rejects a Heze payload with `UnsupportedForkVariant` |
| Explorer | Dora `ghcr.io/lambdaclass/dora:heze-decode`, which decodes the type-`0x06` envelope and shows per-frame receipts |

### Endpoints

| | |
|---|---|
| JSON-RPC | `https://rpc1.privacy.ethrex.xyz` — `eth`, `net`, `web3`, `txpool`, `debug` and `ethrex` (`ethrex_simulateFrameTransaction`, the way to dry-run a validation prefix) |
| Explorer | https://dora.privacy.ethrex.xyz |
| Faucet, EIP guide, frame-tx tooling | https://faucet.privacy.ethrex.xyz (guide at `/eips`) |
| Bootnodes as JSON | https://faucet.privacy.ethrex.xyz/bootnodes — `el` (enodes), `el_enr`, `cl` (beacon ENRs) |
| Genesis bundle | https://faucet.privacy.ethrex.xyz/artifacts/ — `genesis.json`, `config.yaml`, `genesis.ssz`, deposit-contract files, bootnode files, `MANIFEST.txt` with a sha256 per file |
| Beacon API, read-only | https://checkpoint-sync.privacy.ethrex.xyz — `GET /eth/*` on one operator beacon node. It serves a correct finalized block, state and payload envelope; the FOCIL Lighthouse build cannot checkpoint-sync from it yet (see Joining) |

## Consensus inputs the genesis file does not carry

Four values decide which blocks are attestable and appear in no configuration file. A client
that guesses them disagrees silently: there is no state-root mismatch to make it visible.

- **`AA_VOPS_SLOT_COUNT = 4`.** EIP-8369 leaves it unset with a range of 2 to 4. The genesis
  omits the key, and absent means 4, not off.
- **VERIFY budgets are EIP-8369's constants:** `MAX_VERIFY_GAS_PER_IL = MAX_VERIFY_GAS_PER_TX
  = 2**20`, not derived from the 200M block gas limit.
- **Per-inclusion-list code-byte budget:** 16 distinct code bodies and 16 × 64 KiB, charged
  inside the Profile 2 validation-prefix replay, shared across every omitted candidate and both
  evaluation endpoints.
- **Omission eligibility is judged at two fixed endpoints**, `S_start` (parent post-state plus
  the block's pre-execution system operations) and `S_end` (after the last transaction), with
  no builder-claimed insertion index. A strict superset of EIP-8369's default.

All four belong to the EIP-7805 extension that EIP-8369 asks for and that does not exist yet.
Until it does, [`docs/hegota-testnet-joining.md`](hegota-testnet-joining.md) §"Consensus inputs
not present in the genesis file" is the specification.

## Divergences from the pinned text

The nodes run ethrex `31b532266`. Where its behaviour differs from the pins:

| Rule | Live chain | Pinned text | Consequence | Status |
|---|---|---|---|---|
| EIP-8141 `SIGPARAM(0x03)` | returns `len(signature)` for every scheme | ARBITRARY entries only; exceptional halt otherwise | lenient. A pin-exact client rejects blocks whose validation prefix reads it on a secp256k1 entry, and the shielded pool's spends on this chain did (blocks 2787 and 2792) | fixed on the branch at `77e502ef4`; on the chain at the next relaunch |
| EIP-8141 `value_cost` | `TX_VALUE_COST` for every frame with `value > 0` | only when the frame has a target and it is not `tx.sender` | stricter on intrinsic gas: a targetless or self-targeted value frame is overcharged 6,000 | fixed on the branch at `77e502ef4`; on the chain at the next relaunch |
| EIP-8272 `RECENT_ROOT_CODE` | the 144-byte predeploy from [EIPs#12131](https://github.com/ethereum/EIPs/pull/12131) | still `TBD` in the pinned text | none while the PR's bytes hold; a byte change moves the code hash and the write's gas | tracked |
| EIP-8141 `SLOTNUM` in the validation prefix | banned | banned since [EIPs#12066](https://github.com/ethereum/EIPs/pull/12066) merged | conformant | closed |
| EIP-8250 mempool concurrency | a contract sender whose prefix reads no sender storage, installs no code and never touches the legacy nonce may have several keyed transactions pending | one pending frame transaction per sender | mempool policy, not consensus; proposed upstream as [EIPs#12039](https://github.com/ethereum/EIPs/pull/12039) | carried |

The full ledger, with the reasoning behind each row, is
[`docs/hegota-testnet-divergences.md`](hegota-testnet-divergences.md).

## Changed upstream since the pins (not on this chain)

Both changes are consensus-visible and each is a re-genesis to adopt. Tooling written against
this chain must target the pins, not the current drafts.

- **EIP-8272** dropped the envelope field, `TXPARAM 0x11` and `RECENTROOTREFLOAD`. The current
  draft carries the references as a leading VERIFY frame targeting
  `0x0000000000000000000000000000000000008272`, each `(source_id, slot, root)` packed into 72
  bytes of frame data. `RECENT_ROOT_CODE` is still `TBD`.
- **EIP-8250** moved the first use of a keyed nonce from 20,000 execution gas, deducted from
  the frame's remaining gas, to 97,920 state gas charged during the payment `APPROVE` and
  attributed to the frame that calls it. A frame that consumes two fresh nullifier keys
  therefore needs 195,840 of `limits.state` under the current draft; it needs none here.

Diffs of the previous pins to the current ones: [EIP-8141 `4093c21847`→`7d1c8bfb94`](https://gist.github.com/ilitteri/808996324a6409db38f45ed639e82a19),
[EIP-8250 `4093c21847`→`e5cf246ff1`](https://gist.github.com/ilitteri/9f2a53396d92538ef6b6bee380078fbc),
[EIP-8272 `4093c21847`→`0231fb05f5`](https://gist.github.com/ilitteri/7fe0324152d5ae8ca9d888358796337f).

## Testing focus

- **Frame tx lifecycle e2e**: type-`0x06` transactions through the public mempool, per-frame
  receipts (`SUCCESS` / `FAILURE` / `SKIPPED`), `ethrex_simulateFrameTransaction` before
  submission, the explorer decoding every envelope shape (including a signature that omits its
  signer).
- **The pool as sender and payer**: a Groth16 proof verified in a VERIFY frame authorising a
  spend, the pool contract paying for its own transaction, `SIGPARAM` reads on a
  protocol-validated authorizer entry.
- **Keyed-nonce concurrency (EIP-8250)**: two keyed transactions from one contract sender
  admitted side by side and landing in the same block; `NONCE_MANAGER` predeploy reads through
  `TXPARAM 0x0D`–`0x10`.
- **Recent roots (EIP-8272)**: a root written in slot *N* referenceable from slot *N+1*;
  `RECENTROOTREFLOAD` in the prefix; the 144-byte predeploy at `0x…8272`.
- **FOCIL over frame transactions (EIP-7805 + EIP-8369)**: inclusion lists enforced at both
  endpoints with `AA_VOPS_SLOT_COUNT = 4`; `inclusionListSatisfied: false` returned as a `VALID`
  payload the consensus layer must not attest to; engine `newPayloadV6` / `forkchoiceUpdatedV5`
  / `getInclusionListV1`.
- **Mempool admission**: `--mempool.max-verify-gas 500000` (spec default 100,000) so
  proof-carrying prefixes fit; a transaction admitted by one node is re-checked by every node it
  propagates to.
- **Second-client joining**: sync from genesis, PeerDAS custody against three supernodes,
  the ports table, and the FOCIL engine surface at the fork boundary.
- **Shielded pool e2e**: [minimal-shielded-pool](https://github.com/soispoke/minimal-shielded-pool)
  shield → transfer → withdraw → claim on the live chain, at the pins.

### Specs & Tests

**This branch**

- [`docs/hegota-testnet-joining.md`](hegota-testnet-joining.md) — the specification a second client reads: identity, rule set and pins, consensus inputs, artifacts, ports, engine API.
- [`docs/hegota-testnet-divergences.md`](hegota-testnet-divergences.md) — the divergence ledger, every consensus-visible row with its disposition.
- [`docs/hegota-testnet-verification.md`](hegota-testnet-verification.md) — genesis requirements and the post-deploy check pass; `scripts/hegota-testnet/verify_devnet.py` runs it.
- [`docs/hegota-testnet-permissioning.md`](hegota-testnet-permissioning.md) — validator gating policy and the runbook for granting a third party slots.
- [`docs/hegota-testnet-upgrading.md`](hegota-testnet-upgrading.md) — what can change on a live deployment and what is a re-genesis.
- [`scripts/hegota-testnet/INSTALL.md`](../scripts/hegota-testnet/INSTALL.md) and [`USER-GUIDE.md`](../scripts/hegota-testnet/USER-GUIDE.md) — operator install and joiner guide.
- Test suites: `test/tests/levm/eip8141_tests.rs`, `eip8250_tests.rs`, `eip8272_tests.rs`, `test/tests/blockchain/eip8250_concurrency_tests.rs`, `focil_tests.rs`, `focil_profile2_tests.rs`, `focil_eligibility_tests.rs`, `inclusion_list_*_tests.rs`.

**Execution Layer, upstream**

- [EIP-8141 implementation tracker — execution-specs ISSUE-2829](https://github.com/ethereum/execution-specs/issues/2829) Open :exclamation:
- [PR-3047 - Frame Transactions (EIP-8141)](https://github.com/ethereum/execution-specs/pull/3047) Open :exclamation: — the EELS implementation and test suite. No test release covers EIP-8250, EIP-8272 or EIP-8369 yet :hourglass:
- Hive: no simulator for any of the four. Spamoor: no frame-tx scenario :hourglass:

### Open PRs

**EIPs**

- [PR-12110 - EIP-8369 VOPS profiles for FOCIL eligibility](https://github.com/ethereum/EIPs/pull/12110) Open :exclamation: — implemented here at `33724bd7da`
- [PR-12131 - specify `RECENT_ROOT_CODE`](https://github.com/ethereum/EIPs/pull/12131) Open :exclamation: — the bytes this chain runs; a change is a fork
- [PR-12041 - canonical paymaster reference bytecode](https://github.com/ethereum/EIPs/pull/12041) Open :exclamation: — implemented ahead of merge
- [PR-12039 - keyed mempool concurrency](https://github.com/ethereum/EIPs/pull/12039) Open — the mempool extension this chain ships
- Merged and conformant: [PR-12066](https://github.com/ethereum/EIPs/pull/12066) `SLOTNUM` ban, [PR-12109](https://github.com/ethereum/EIPs/pull/12109) atomic-batch approval scope, [PR-12026](https://github.com/ethereum/EIPs/pull/12026) floor repricing, [PR-12061](https://github.com/ethereum/EIPs/pull/12061) frame receipt status, [PR-12062](https://github.com/ethereum/EIPs/pull/12062) state-gas dimension, [PR-12113](https://github.com/ethereum/EIPs/pull/12113) initial access set :heavy_check_mark:

**Tooling**

- [minimal-shielded-pool PR-1 - move the tooling to the EIPs at this chain's pins](https://github.com/soispoke/minimal-shielded-pool/pull/1) Open
- [ethrex PR-6974 - `ethrex_simulateFrameTransaction`](https://github.com/lambdaclass/ethrex/pull/6974) — the dry-run RPC; live on this chain's nodes

## Joining

No validator needed. Fetch the whole bundle from `/artifacts/` into one directory, keep the
filenames, and check it against `MANIFEST.txt`; a consensus client is pointed at that directory
as a whole (Lighthouse: `--testnet-dir`).

```
ethrex --network genesis.json \
       --bootnodes "$(paste -sd, bootnodes.txt)" \
       --nat.extip <your public IP> \
       --syncmode full

lighthouse bn --testnet-dir <bundle dir> \
       --boot-nodes "$(paste -sd, bootnodes-cl.txt)" \
       --allow-insecure-genesis-sync \
       --execution-endpoint http://localhost:8551 --execution-jwt <jwt>
```

- **Sync from genesis.** The chain imports at roughly 20 slots per second, so it is minutes.
  The read-only beacon endpoint serves correct checkpoint data, but `lighthouse:focil` does not
  fetch the anchor's execution payload envelope on a Gloas chain, so a checkpoint-synced node
  never leaves the checkpoint slot (`Block has an unknown parent`).
- **The consensus client must be FOCIL-aware** from the fork boundary: ethrex rejects
  `engine_newPayloadV5` and `engine_forkchoiceUpdatedV4` once Hegotá is active. Verify
  `engine_getInclusionListV1`, `engine_newPayloadV6`, `engine_forkchoiceUpdatedV5` and
  EIP-7843 `slotNumber` through `engine_exchangeCapabilities` before epoch 2.
- **Ports:** discovery needs TCP *and* UDP open. Operator nodes listen on 32000/32007/32014
  (EL) and 31000/31007/31014 (CL, plus QUIC one stride above). `--nat.extip` is what a node
  advertises; `--p2p.addr` is only the bind address.
- **Expect one benign error** at consensus-client start: `Failed bucket filter` for the third
  CL bootnode. All three share one IP and discv5 admits two per /24 into a bucket; the node is
  still dialed.
- **Validator entry** is a separate, permissioned step: a deposit needs an access token held
  by the depositing address and `0x01`/`0x02`/`0x03` withdrawal credentials (BLS `0x00` is
  blocked). Client teams working on frames or FOCIL who want to validate: reach out and we
  will issue deposit tokens.

## Local testing

The full kurtosis file is
[`fixtures/networks/hegota-testnet.yaml`](../fixtures/networks/hegota-testnet.yaml); use it
rather than retyping the example, because it also carries the EIP-8282 builder-predeploy
inhibitor the genesis generator omits (without it `getPayload` fails from the Amsterdam
boundary on) and the gated-deposit settings. Launch with the pinned package,
`github.com/ethpandaops/ethereum-package@b5b3af65248f11702216e377d0377bcd8ccf4caf`, and an
`ethrex:hegota-testnet` image built from the branch (`scripts/hegota-testnet/INSTALL.md`).

```yaml
participants:
  - el_type: ethrex
    el_image: ethrex:hegota-testnet
    cl_type: lighthouse
    cl_image: ethpandaops/lighthouse:focil
    el_extra_params:
      - --mempool.max-verify-gas=500000
      - --http.api=ethrex
      - --http.api=txpool
      - --http.api=debug
    validator_count: 32
    supernode: true

network_params:
  network_id: "8141"
  seconds_per_slot: 6
  fulu_fork_epoch: 0
  gloas_fork_epoch: 1     # must be scheduled, and not at 0
  heze_fork_epoch: 2

ethereum_genesis_generator_params:
  image: ethpandaops/ethereum-genesis-generator:6.1.6

additional_services:
  - dora
dora_params:
  image: ghcr.io/lambdaclass/dora:heze-decode
```

---

Looking for the frames-only devnet? https://notes.ethereum.org/@ethpandaops/frames-devnet-0

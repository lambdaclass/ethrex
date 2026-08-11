# Hegotá testnet — joining

Everything a second client needs to follow this chain. It is written for someone with
no access to this repository and no knowledge of ethrex: if a rule is not stated here,
it is not derivable from the genesis file either.

**Joining does not require a validator.** Anyone may sync, peer and submit
transactions. Only *validator entry* is permissioned, through a gated deposit
contract; see `docs/hegota-testnet-permissioning.md`. That asymmetry is the Sepolia
model, and it is what makes this network simultaneously open to a third party and
controlled by its operators.

## Identity

| | |
| --- | --- |
| Chain ID / network ID | `8141` |
| Deposit contract | `0x00000000219ab540356cBB839Cbe05303d7705Fa` |
| Deposit gater | `0x00000000a11acc355c0de0000a11acc355c0de00` |
| Seconds per slot | 6 |

Chain ID `8141` is unclaimed in the `chainid.network` registry and is deliberately not
the kurtosis default `3151908`, which every local devnet collides on.

The deposit contract keeps the canonical mainnet address on purpose. The gated
contract is the mainnet deposit contract plus one gater call; it emits the same
`DepositEvent` with `DEPOSIT_TOPIC`
`0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5` and no additional
event, so it is transparent to an execution layer that matches deposit logs by address
and topic.

## The rule set

**Do not implement this fork from its name.** EIP-8081 (*Hardfork Meta - Hegotá*)
schedules EIP-7805 alone under that name; this chain activates five EIPs at the same
timestamp. A client that implements "Hegotá" from the meta EIP implements FOCIL only
and rejects every frame transaction on the chain. The authoritative statement of what
this network runs is the list below, pinned to exact revisions.

| EIP | Title | Pin | Source |
| --- | --- | --- | --- |
| 8141 | Frame Transaction | `4093c21847` | `ethereum/EIPs` |
| 8250 | Keyed Nonces | `4093c21847` | `ethereum/EIPs` |
| 8272 | Recent Roots | `4093c21847` | `ethereum/EIPs` |
| 7805 | Fork-choice enforced Inclusion Lists (FOCIL) | `4093c21847` | `ethereum/EIPs` |
| 8369 | VOPS Profiles for FOCIL Eligibility | `6f818e27dd` | `soispoke/EIPs@codex/vops-profiles-focil` (PR #12110, unmerged) |

All five activate together at one timestamp. There is no per-EIP activation and no
intermediate state in which some are live and others are not.

EIP-8369 is in the set because EIP-7805 enforcement over frame transactions is
undefined without an eligibility rule, and EIP-8369 is that rule. It activates with
the fork exactly like the other four; what it does not have is an independent off
switch, which has a consequence stated under "Consensus inputs" below.

Also relevant, and inherited rather than chosen by this chain: the fork sits on top of
Amsterdam (EIP-7773), and every EIP in that meta is live, including EIP-7928
block-level access lists, EIP-8037 state-creation gas and EIP-8282 builder execution
requests. A client that does not implement Amsterdam cannot follow this chain at any
height.

Two EIPs are present in the ethrex binary but **not** part of this chain's rule set:
EIP-8312 (UTXO frames) is inert because `utxoFramesTime` is unset, and EIP-7906
(transaction assertions) is deleted from the branch entirely. Neither should be
implemented.

## Fork schedule

The consensus layer names the fork `heze`; the execution genesis names the same
timestamp `bogotaTime`. ethrex reads `hegotaTime`, `hezeTime` and `bogotaTime` as
aliases for one field (`crates/common/types/genesis.rs:284-296`).

| Fork | Epoch | Timestamp |
| --- | --- | --- |
| Fulu / Osaka | 0 (genesis) | genesis |
| Gloas / Amsterdam | 1 | genesis + 192s |
| Heze / Hegotá / Bogotá | 2 | genesis + 384s |

At 6-second slots and 32 slots per epoch, epoch 1 is 192 seconds after genesis and
epoch 2 is 384. The published `genesis.json` carries the resolved absolute values;
prefer them over recomputing.

Gloas must be scheduled and must not be at epoch 0. Hegotá is defined on top of it,
and a beacon genesis state built at Gloas is rejected by the client that consumes the
generator's output.

## Consensus inputs not present in the genesis file

Four values decide consensus outcomes on this chain and appear in no configuration
file. A client that guesses them will disagree about which blocks are attestable, and
that disagreement produces no state-root mismatch to make it visible.

**`AA_VOPS_SLOT_COUNT = 4`.** EIP-8369 leaves the value unset with a candidate range
of 2 to 4. This chain uses 4. The genesis file omits the key, and **absent means 4,
not disabled**: there is no way to turn Profile 2 off. 4 is the top of the range and
therefore the permissive choice, since eligible-at-4 is a superset of eligible-at-2.

**VERIFY budgets are EIP-8369's constants, not derived.**
`MAX_VERIFY_GAS_PER_IL = MAX_VERIFY_GAS_PER_TX = 2**20`. This is worth stating because
the arithmetic invites a different answer: this chain's block gas limit is
200,000,000, and deriving the budget from it yields roughly 3× the constant and puts
committee replay at a quarter of the block. The constant is what this chain uses.

**Per-inclusion-list code-byte budget: 16 distinct code bodies and 16 × 64 KiB.**
Charged inside the Profile 2 validation-prefix replay, as each body is loaded, ending
the replay on the first body that does not fit. The allowance is per inclusion list:
shared across every omitted candidate and across both evaluation endpoints, with an
already-charged `codeHash` free to load again. Charges survive a failed verdict,
because a replay that loaded bodies and then failed still made every attester read
them. The charge order is list order over the omitted candidates only. EIP-8369
specifies no such bound and asks the enforcing extension to supply one; a client with
a different bound reaches a different verdict on the same list.

**Omission eligibility is judged at two fixed endpoints.** An omission is unjustified
when the candidate is eligible at `S_start` **or** at `S_end`, where `S_start` is the
parent post-state with the block's pre-execution system operations applied and `S_end`
is the state after the block's last transaction. There is no builder-claimed insertion
index. `S_end` alone is exactly EIP-8369's default index, so this rule is a strict
superset: it can reject a block a spec-literal client accepts, never the reverse.
`gas_fits` is evaluated at `S_end` only, because gas remaining is monotone within a
payload.

These four are the subject of a Standards Track extension to EIP-7805, which EIP-8369
explicitly asks for and which does not yet exist. Until it is published, this section
is the specification.

## Divergences from the pinned text

Two, both one-way in the direction of ethrex being stricter, so neither can cause
ethrex to accept a block a spec-literal client rejects.

- **`SLOTNUM` (`0x4B`) is banned in the EIP-8141 validation prefix.** Upstream
  EIP-8141's banned list does not yet include it; `ethereum/EIPs#12066` is the open PR
  that adds it. The ban is fork-choice visible on this chain through FOCIL, because a
  banned opcode in a prefix makes a transaction ineligible, which makes its omission
  justified, which decides whether a payload satisfies its inclusion lists.
- **The EIP-8272 recent-root predeploy at `0x…8272` runs the 144-byte
  `RECENT_ROOT_CODE`** from `ethereum/EIPs#12131`, which is unmerged. ethrex's copy is
  byte-identical to that PR. There is no native write path and no fixed gas constant;
  the cost is whatever the EVM charges for executing the predeploy.

## Engine API

FOCIL forces the engine API to the V6/V5 pair. ethrex rejects `engine_newPayloadV5`
and `engine_forkchoiceUpdatedV4` once Hegotá is active, because only V6/V5 carry
`inclusionListTransactions`. A consensus client must provide:

- `engine_getInclusionListV1`
- `engine_newPayloadV6`
- `engine_forkchoiceUpdatedV5`
- EIP-7843 `slotNumber` on the payload attributes

Verify all four through `engine_exchangeCapabilities` **before** the fork boundary.
There is no inert intermediate state: execution and consensus upgrade together, and a
client that speaks only V4/V5 halts the node at the boundary.

An unsatisfied inclusion list is reported as a `VALID` payload carrying
`inclusionListSatisfied: false`. The block is valid; the consensus layer simply must
not attest to it. It is not an `INVALID` status and not a JSON-RPC error.

## Artifacts

`scripts/hegota-testnet/publish-artifacts.sh` extracts the bundle from a running
enclave. Fetch every file and check it against `MANIFEST.txt`, which carries a sha256
per file.

| File | Purpose |
| --- | --- |
| `genesis.json` | geth-style execution genesis; ethrex and geth consume this |
| `chainspec.json` | Nethermind-format execution genesis |
| `besu.json` | Besu-format execution genesis |
| `config.yaml` | consensus-layer config |
| `genesis.ssz` | beacon genesis state |
| `deposit_contract_block_hash.txt` | deposit contract deployment block hash |
| `genesis_validators_root.txt` | beacon genesis validators root |
| `bootnodes.txt` | three execution-layer `enode://` URLs |
| `bootnodes-cl.txt` | three beacon-node ENRs |
| `MANIFEST.txt` | sha256 of every file above |

### A note on `chainspec.json`

The genesis generator emits per-EIP transition keys for the Nethermind format, and at
generator `v6.1.6` it writes only `eip7805TransitionTimestamp` and
`eip8141TransitionTimestamp` for this fork. `publish-artifacts.sh` adds
`eip8250TransitionTimestamp` and `eip8272TransitionTimestamp` at the same value.

**The key names must be confirmed with Nethermind before relying on them.** They are
this repository's guess at the naming convention, not a name any Nethermind release is
known to read. The authoritative activation statement is the fork timestamp plus the
five-EIP set above; the chainspec keys are a convenience for one client's
configuration parser and nothing in this chain's definition depends on them. ethrex
does not read them at all: it takes `bogotaTime` from `genesis.json` through the
`hezeTime` / `bogotaTime` aliases.

## Ports and firewall surface

The package allocates 7 ports per execution node and 7 per consensus node
(`shared_utils.MAX_PORTS_PER_EL_NODE`, `MAX_PORTS_PER_CL_NODE`), striding from the
configured start: index 0 is discovery TCP+UDP, 1 is engine/HTTP, 2 is metrics, and
for an execution node 3 is JSON-RPC. With `el.public_port_start: 32000`,
`cl.public_port_start: 31000` and `additional_services.public_port_start: 31500`:

| Purpose | Ports | Proto | Exposure |
| --- | --- | --- | --- |
| EL discv4 + RLPx | 32000, 32007, 32014 | TCP **and** UDP | must be public for peering |
| CL discv5 + libp2p | 31000, 31007, 31014 | TCP **and** UDP | must be public for peering |
| EL JSON-RPC (node 0 only) | 32003 | TCP | public via reverse proxy |
| Dora explorer | 31500 | TCP | public via reverse proxy |
| EL engine authrpc | 32001, 32008, 32015 | TCP | must stay closed |
| EL metrics | 32002, 32009, 32016 | TCP | must stay closed |
| EL JSON-RPC (nodes 1, 2) | 32010, 32017 | TCP | must stay closed |
| CL beacon REST | 31001, 31008, 31015 | TCP | must stay closed |
| CL metrics | 31002, 31009, 31016 | TCP | must stay closed |

The discovery ports need **both** TCP and UDP. A rule that opens only TCP produces a
node that accepts inbound RLPx from peers that already know it and is never
discovered by anyone else, which reads as slow peering rather than as a firewall
error.

The JSON-RPC and explorer ports are served through a reverse proxy rather than opened
directly, following `scripts/eip8141-devnet/Caddyfile`. The engine ports carry the
JWT-authenticated payload API; reaching them is equivalent to controlling the node's
head, and they must never be publicly reachable.

Every published port sits below 32768, the default floor of
`/proc/sys/net/ipv4/ip_local_port_range`. Kurtosis hands dynamic host ports to
unpublished services out of that range, so a fixed publish inside it races them and
can lose with `failed to bind host port … address already in use`. On a host doing NAT
the two constraints compose: every port an external peer must reach has to be both
fixed and below the floor.

## Starting a node

Execution layer, ethrex:

```
ethrex --network genesis.json \
       --bootnodes "$(cat bootnodes.txt | paste -sd,)" \
       --nat.extip <your public IP> \
       --syncmode full
```

`--nat.extip` is what the node advertises in discovery and in its ENR; `--p2p.addr` is
the bind address and is not a substitute. A node that omits it advertises whatever
local address it found and no external peer can dial back.

Consensus layer:

```
<client> --boot-nodes "$(cat bootnodes-cl.txt | paste -sd,)"
```

with `config.yaml` and `genesis.ssz` from the bundle. The client must be FOCIL-aware;
see "Engine API" above.

## Becoming a validator

Separate, and permissioned. Depositing requires an access token held by the depositing
address; the deposit reverts with "Not enough tokens" without one. BLS (`0x00`)
withdrawal credentials are blocked outright, so a deposit must use execution-layer
(`0x01`), compounding (`0x02`) or builder (`0x03`) credentials. Top-ups to an existing
validator need no token.

`docs/hegota-testnet-permissioning.md` holds the policy and the runbook for granting a
third party validator slots.

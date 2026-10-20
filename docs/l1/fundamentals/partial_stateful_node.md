# Partial stateful node

## What it is

`ethrex partial-stateful-node` (alias `ethrex psn`) runs ethrex as a **partial stateful
node**: an execution-layer node that holds only the accounts and storage its users read, proves
every value it serves against headers a light client verified, and follows the chain by applying
EIP-7928 block access lists to the keys it holds instead of executing blocks. It is built for
wallets: a local JSON-RPC endpoint that is ready in minutes, keeps kilobytes on disk, runs in
tens of megabytes of memory, and trusts no RPC provider. The design follows [chibi](https://github.com/weiihann/chibi), with
[EIP-8440 execution chain proofs](https://github.com/ethereum/EIPs/pull/12464) layered on top.
It is optional: build with `--features partial`; the implementation is the `ethrex-partial`
crate in `tooling/partial`.

### Where it sits

| | Full node | Partial stateful node | Wallet on an RPC provider |
|---|---|---|---|
| State held | all of it | the keys its users touch, each one proven | none |
| Follows the chain by | executing every block | applying block access lists to held keys | asking the provider |
| Trusts | genesis and its own execution | a checkpoint and the sync committee, plus chain proofs when they exist | the provider |
| Ready in | hours to days | minutes, waiting for a peer | at once |
| Disk, memory | hundreds of GB, gigabytes | kilobytes, tens of megabytes | none |
| Serves | every JSON-RPC method | the wallet subset: balance, nonce, code, storage, call, estimate, fees, send, its own receipts | whatever the provider serves |

### What it is not

- **Not a consensus client.** It embeds a light client, the verifying part of one, and reads a
  beacon node's REST API for the sync committee data that light client checks. It follows no
  beacon gossip and runs no validator duties. A beacon node is a requirement, a consensus client
  process on the same machine is not; see [What it needs](#what-it-needs).
- **Not an indexer or an archive.** It holds no logs, filters, history or full blocks, and
  serves state at the anchor and the eight blocks before it.
- **Not a substitute for validating the chain yourself.** It trusts the sync committee, and
  chain proofs when a network has them, instead of executing. A full node is still what you
  run when that trust is not acceptable.

### Trust in one sentence

Every value it serves descends from a header the sync committee signed: snap proofs against
that header's state root, access lists committed to by its hash, bodies and receipts committed
to by its roots, and, when present, an EIP-8440 proof that the execution up to it was valid. A
lying peer or beacon node can stall the node; it cannot make it serve a wrong value.

## What it needs

Four things from outside the node, and very little hardware.

### A beacon node, not a consensus client

Consensus clients expose a standard REST interface, the
[Beacon API](https://ethereum.github.io/beacon-APIs/): headers, blocks, the chain's
configuration, an event stream, and the light client endpoints. The partial node reads the last
group: `light_client/bootstrap` once at the checkpoint, `light_client/updates` once per sync
committee period, and `light_client/finality_update` and `light_client/optimistic_update` every
slot, plus `headers`, `genesis` and `config/spec`. That is what `--beacon.url` points at.

It does not run a consensus client itself, and does not need one on the same machine. It embeds
a light client, the verifying part of one: every header it takes is checked against the sync
committee's aggregate BLS signature, every committee change against its Merkle branch from the
state the previous committee signed. The beacon node can censor or stall the light client; it
cannot make it accept a header the committee did not sign. Any beacon node that serves light
client data works, local or remote. Lighthouse, Nimbus, Lodestar and Teku serve it; a node that
does not serve it at all answers `404` on every light client route, which the log reports.

### Execution peers

Ordinary execution clients over devp2p, found through the network's bootnodes or `--bootnodes`.
Headers, bodies, receipts and block access lists come over `eth/71`; the state proofs come over
`snap`, as single-key account and storage range requests, and block access lists over `snap/2`
where a peer offers it. ethrex and geth both serve what is needed. The first anchor waits for one
peer that serves state and a few that speak `eth` (`--min-eth-peers`), and then for one of them
to answer the first header request; on Sepolia today that is where the minutes go.

### A network past Glamsterdam

Two pieces of the design are Glamsterdam features:

- **EIP-7928 block access lists (Amsterdam)** are how the node advances without executing. A
  header before Amsterdam carries no access-list hash, so on such a network every step falls back
  to reanchoring: the pins are re-proven each block and the learned keys are dropped. Correct,
  but slow, and `eth_call` warmth resets every slot.
- **Gloas light client headers** carry the hash of the execution block the beacon chain applied,
  which is what the follower anchors on, and the EIP-8440 proof machinery is defined on Gloas
  payload envelopes.

Sepolia activated Glamsterdam in October 2026 and is where the node is exercised. Mainnet has
no Amsterdam activation scheduled yet; nothing in the node is network-specific, so
`--network mainnet` with a mainnet beacon node becomes the target the moment it forks. Pre-Gloas
light client headers are handled, so the light client itself bootstraps on any network since
Altair.

### A checkpoint you trust

`--checkpoint` is a finalized beacon block root: the weak-subjectivity checkpoint, the one
thing the light client takes on faith. Without it the node asks the beacon node for its
finalized block and says so in the log, which makes that node the trust anchor; fine for a
test, not for a wallet. Any block explorer or beacon node you trust gives you the root.

The beacon node must hold light client data for that root. Lighthouse keeps it only for
finalized checkpoint roots whose block sits at an epoch boundary; when the boundary slot was
skipped, the checkpoint root is an earlier block and `light_client/bootstrap` answers `404`
for it. The node retries for half a minute and then exits with the route and the status; pick
the previous finalized checkpoint, or wait an epoch.

### Hardware

Anything that runs the ethrex binary. The node holds no database, executes no blocks, and
keeps a bounded working set, so its footprint is that of the networking stack and the light
client. Measured on Sepolia in `off` mode on 2026-10-20, over six and a half minutes of
following the chain with one watched account:

| | Measured | Why it stays there |
|---|---|---|
| Memory | 29 to 34 MB resident | the working set is capped at 1024 keys and eight recent states; headers are kept for 256 blocks; a chain proof is at most 4 MiB and is dropped once verified |
| CPU | 3% of one core on average, brief peaks under 10% | one BLS aggregate check per slot, a few Merkle and trie proofs per block, levm only for `eth_call` and `eth_estimateGas` |
| Disk | 8 KB in the data directory, plus the 40 MB binary | only the node key and its ENR are written; state is never persisted |
| Network | a few kilobytes per second each way | headers and access lists over devp2p, light client updates over HTTPS; a reanchor adds one snap proof per held key |
| Time to serve | light client bootstrapped in 5 s; peers connected in 25 s; first anchor at 4 min | the wait is for a peer that answers the first header request, and Sepolia has few that serve this node |

The two cache lines the startup log prints, a RocksDB block cache in the gigabytes and a
bytecode cache of 2 GiB, are the full node's store sizing itself; the partial node's store
is in memory and headers-only, and neither cache is allocated to that size, as the resident
memory above shows.

A single core and 128 MB of memory leave headroom; a Raspberry Pi, a NAS or a phone-class
board is enough. Bandwidth matters more than compute: the node is idle between slots and
busy for a fraction of a second at each one.

### Optionally, a source of EIP-8440 proofs

With `--proof-mode observe` or `require` the node also reads execution chain proofs from
`GET /eth/v1/beacon/execution_proofs/{block_id}` on `--proofs.url`, which defaults to the
light client's node. The two can differ: an ethlambda node serves verified proofs but no light
client data today, so it pairs with a Lighthouse or Nimbus for the latter. No public network
produces chain proofs yet, which is why the default mode is `off`; see
[EIP-8440 execution chain proofs](#eip-8440-execution-chain-proofs).

## Running

```sh
cargo build --release --features partial --bin ethrex

./target/release/ethrex partial-stateful-node \
  --network sepolia \
  --beacon.url https://beacon.sepolia.ethpandaops.io/nimbus \
  --checkpoint 0xa8d991bb5d16328401a8b10568b85dcb60bddf62053547069929bab00346a8a7 \
  --watch 0xYourAddress,0xAnotherAddress \
  --http.port 8545
```

Point a wallet at `http://127.0.0.1:8545`. The node serves JSON-RPC once it is anchored at
the light client's head and a peer serving state (snap) is connected.

| Flag | Meaning |
|---|---|
| `--beacon.url URL` | Beacon REST API the light client reads sync committee data from. It can stall the node but cannot make it accept an unsigned header. |
| `--checkpoint ROOT` | Weak subjectivity checkpoint: a finalized beacon block root you trust. Without it the beacon node's finalized block is trusted as the starting point, and the node says so in its log. Beacon nodes publish light client data for a fresh checkpoint a little after finalizing it, so a missing bootstrap is retried; a node-chosen checkpoint also falls back to the previous epoch's. |
| `--watch ADDR,...` | Accounts kept proven at every head (pins). Everything else is learned from use. |
| `--working-set-cap N` | Most keys kept proven, pins included (default 1024, LRU eviction). |
| `--proof-mode off\|observe\|require` | How EIP-8440 chain proofs are used (see below). |
| `--proof-engine.url URL` | The proof engine that verifies chain proofs. |
| `--proof-engine.mock` | Accept mock proofs without verification. Devnets and tests only. |
| `--proofs.url URL` | Beacon REST API to read proofs from when it is not the light client's node, such as one that serves proofs but no light client data. Defaults to `--beacon.url`. |
| `--min-eth-peers N` | Peers speaking eth to wait for before anchoring (default 3), so header requests have somewhere else to go when one peer cannot serve them. |
| `--network`, `--datadir`, `--p2p.*`, `--bootnodes`, `--log.level`, `--http.*` | Shared with a full node. |

The data directory holds the node key and the learned keys (`partial/keys.json`). Proven
state lives in memory and is re-proven on restart.

### Example

```
$ curl -s -X POST localhost:8545 -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0xYourAddress","latest"]}'
{"jsonrpc":"2.0","id":1,"result":"0x…"}

$ curl -s -X POST localhost:8545 -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"partial_status","params":[]}'
{"jsonrpc":"2.0","id":1,"result":{"anchor":{"number":11875419,"hash":"0x…","timestamp":1760900000},
 "chainProofs":{"mode":"observe","verified":12,"provenPayloads":12,"anchorProven":true},
 "held":{"accounts":41,"slots":117,"pins":3,"learned":155},"peers":{"state":2,"eth":9}, …}}
```

### JSON-RPC served

`web3_clientVersion`, `net_version`, `eth_chainId`, `eth_blockNumber`, `eth_syncing`,
`eth_getBlockByNumber`, `eth_getBlockByHash` (headers only; no transactions), `eth_getBalance`,
`eth_getTransactionCount` (`pending` counts submitted transactions), `eth_getCode`,
`eth_getStorageAt`, `eth_call`, `eth_estimateGas`, `eth_gasPrice`, `eth_maxPriorityFeePerGas`,
`eth_feeHistory`, `eth_sendRawTransaction`, `eth_getTransactionReceipt` and
`eth_getTransactionByHash` (transactions this node submitted), and `partial_status`.

State is served at `latest`/`pending` (the anchor) and at the eight blocks before it. Logs and
filters are not served: the node holds no bodies or receipts except for its own transactions.

## How it works

```
 beacon API ──► light client ──► TrustedHash(execution block) ──► header (devp2p, hash-checked)
                     │                                                   │
                     │  beacon roots                                     ▼
                     ▼                                     ┌─────────────────────────────┐
             EIP-8440 chain proofs ──► proof engine ──►    │  Follower                   │
             (origin ≤ checkpoint)      VALID/INVALID      │  extend: BALs (eth/71, snap/2)│
                                                           │  reanchor: snap proofs (pins)│
                                                           └──────────────┬──────────────┘
                                                                          ▼
   wallet ──► JSON-RPC ──► Node ──► pinned Store ──► levm (PartialDb) ──► misses ──► snap proofs
```

### Trust model

| Layer | Trust source | What it establishes |
|---|---|---|
| Light client | the weak-subjectivity checkpoint root and an honest sync committee | which beacon block is head or finalized, and its applied execution block hash (Gloas light client header) |
| EIP-8440 chain proof | the proof system and the proof engine; the origin is at or before the checkpoint | every payload from the origin to the head executed validly and is bound to the beacon chain |
| snap proofs | the state root of a verified header | an account, a slot, or its absence, at the anchor |
| block access list | `block_access_list_hash` in a verified header | post-values of held keys for the next block |
| bodies and receipts | the header's roots and logs bloom | inclusion and receipts of the node's own transactions |

Every value the node serves descends from one of these. A key never fetched is a miss, never
a zero; only a key proven absent reads as zero. A lying peer can stall the node but cannot
make it serve a wrong value.

### Following the chain

The light client (`light_client/`) runs the Altair sync protocol with the Electra and Gloas
generalized indices: it bootstraps from the checkpoint root, follows sync committee periods,
and polls the finality and optimistic updates once per slot. Each verified header carries the
execution block hash the beacon block applied, which becomes the node's `TrustedHash`.

The follower (`follow.rs`) fetches that header from devp2p peers and checks its hash. To move
the anchor from block `N` to `N+k` (`k ≤ 32`) it fetches the block access lists of the
`k` blocks (snap/2 `GetBlockAccessLists`, then eth/71), checks each against its header's
`block_access_list_hash`, and applies the post-values to the keys it holds. Nothing is
executed. A longer gap, a reorg, or a withheld access list makes it **reanchor**: prove the
pins at the new head with snap proofs, drop learned keys, and warm them back afterwards.

### Proving state

Proofs are single-key snap range requests (`GetAccountRange` and `GetStorageRanges` with
origin and limit equal to the key), verified by rebuilding the proof nodes into an
authenticated trie and walking the key (`verify.rs`). A walk that ends at a leaf proves
presence, one that ends short proves absence, and a proof that carries a leaf the reply left
out is a lie and costs the peer its standing. Storage roots are never held because access
lists would leave them stale, so every slot proof re-proves its account in the same round.

`eth_call` and `eth_estimateGas` run levm over the pinned store through a database that
records what it lacks. A missing value reads as empty for that run only and the result is
withheld; the node proves the misses at the same anchor and runs again. Round trips scale
with dependency depth, not key count. Every key read is learned, so it stays warm at later
heads.

### EIP-8440 execution chain proofs

An execution chain proof attests that every payload from its origin beacon block to its head
beacon block executed validly. A node that starts from a checkpoint and sees a verified proof
whose origin is at or before that checkpoint has established valid execution up to the proof's
head, without executing anything. This is what lifts the node's trust in the state it serves
from "the sync committee signed this header" to "this header's execution is proven".

The node maps proofs to execution blocks with the Gloas light client headers: for consecutive
headers `N` and `N'`, `N'.execution_block_hash != N.execution_block_hash` means `N` was full
and committed the payload `N'.execution_block_hash`, so a proof with `head_block_root = root(N)`
proves that execution block (`chain_proofs/tracker.rs`). Proofs are fetched from the beacon
node (`GET /eth/v1/beacon/execution_proofs/{block_root}`) and verified through a proof engine
against `hash_tree_root(PublicInput{origin, head, chain_id, schema_id})`.

| Mode | Behaviour |
|---|---|
| `off` | Chain proofs are not used. The anchor is the light client's head. |
| `observe` | Proofs are fetched and verified; `partial_status` reports how many execution blocks are proven and whether the anchor is. The anchor is the light client's head. |
| `require` | The anchor starts at the checkpoint's execution block, the trust root, and only moves to execution blocks with a verified chain proof whose origin is at or before the checkpoint. Reads at `latest` are served there. |

In `require` mode the checkpoint and the prover's origin have to agree: a proof whose origin is
after the checkpoint says nothing about the gap between the two and is refused, so with a
prover that started recently the anchor never moves and `partial_status` reports `verified`
at zero. The node warns once per such origin, naming both slots; the fix is a checkpoint at or
after the origin.

The proof engine is reached over HTTP as `POST /v1/execution_proof_verifications` with the
proof bytes as the body and the public input fields as query parameters
(`origin_block_root`, `head_block_root`, `chain_id`, `schema_id`, `proof_type`), the contract
of Lighthouse's EIP-8025 proof-engine sidecar client with the EIP-8440 fields in place of the
EIP-8025 ones. No EIP-8440 engine exists yet; the parameter names are this node's assumption
and live in one place (`chain_proofs/engine.rs`). Proofs are read from the beacon node's
`GET /eth/v1/beacon/execution_proofs/{block_id}`, the route the eth-act Lighthouse branch
serves for EIP-8025 proofs; Prysm publishes received proofs on its event stream instead
(`/eth/v1/events?topics=execution_proof`), which this node does not read yet. The prover's
signature is not checked: it only gates gossip, and a proof's validity does not depend on who
signed it.

**Where consensus clients are (2026-10-20).** Prysm (OffchainLabs/prysm#17690) and the eth-act
Lighthouse branch `optional-proofs` implement EIP-8025, single-payload proofs, with the ERE
zkVM verifiers embedded in the client and the ethrex stateless-validator guests as proof types
1 to 3; upstream Lighthouse verifies through the external HTTP sidecar above. ethlambda takes
EIP-8440 proofs from gossip and its Beacon API, verifies them through a proof engine (today a
mock), keeps them until finality and serves them on the `GET` route above; its validator client
can sign a mock chain proof per slot for devnets, which is what this node was run against (see
[Live run](#live-run)). No client produces real EIP-8440 chain proofs: the recursive chain guest
does not exist yet, so `observe` and `require` can only be exercised against the mock engine
until it does. Covering the gap with one EIP-8025 proof per payload is possible (ethrex already
has the `NewPayloadRequest` SSZ container) but is not implemented.

## What is ethrex's and what is new

The crate is larger than the idea suggests because the pieces ethrex already has were built
for a node that holds the whole state, and three of the five layers below needed a version
that does not. Everything reused is linked, not copied; everything new is listed with the
reason it could not be reused and, where there is one, the refactor that would let it be.

| Layer | From ethrex | New here | Why |
|---|---|---|---|
| Networking | the whole devp2p stack: discovery, `PeerTable`, `PeerHandler`, the `eth` and `snap` message types, header, body, receipt and access list requests | `p2p_adapter.rs`, `fetch.rs`, `peers.rs`: a `Peers` trait over it, single-key snap requests, per-anchor retries | the full node's snap client asks for ranges to fill a trie; this node asks for one key at a time, at a specific header |
| Verification | `ethrex-trie` nodes and hashing, header, body, receipt and access list hashing, RLP | `verify.rs`: proof-node walks for one key, the pin and absence rules, the anchor chain | `verify_range` trusts the requester to have chosen the range's bounds; here the peer chooses them, and a withheld left-bound leaf would prove a false absence, so the proof is walked from the root to the key instead |
| State | the account, storage and code types; the EIP-7928 access list types and hash | `state.rs`, `working.rs`: the pinned store, access list application, reanchoring, retention of the last eight blocks | ethrex's store is a complete state trie with a root; this one is a set of proven keys with the block each was proven at |
| Execution | levm, whole, through its `VmDatabase` trait; the gas cap and fee rules | `exec.rs`: a database that records the keys levm reads and misses, and the round loop that proves them and runs again | levm needed no change; the loop around it is the partial part |
| JSON-RPC | request and batch parsing, the response and error shapes, the block, transaction and receipt JSON types | `rpc.rs`: the `eth_*` dispatch over the pinned state, `partial_status` | ethrex's handlers take the full node's context (its store, blockchain and sync state); a handler that takes a state-provider trait would let both nodes share them |
| Light client | nothing: ethrex has no consensus-layer code | `light_client/`: the beacon containers, the sync protocol, Merkle branches, BLS over `blst`, the Beacon API client | see below |
| Chain proofs | the stateless-input schema id and SSZ helpers the ethrex guests use | `chain_proofs/`: the EIP-8440 containers, the proof source client, the mock and HTTP engines, the tracker | ethrex produces proofs; nothing in it consumed one |

Tests are a third of the crate, and `testing.rs` is most of that: an in-process peer that
answers snap, `eth` and access list requests from a real `ethrex-blockchain` and
`ethrex-storage` store, so the follower, the verifier and the executor are tested against the
same code a full node serves with.

### Why the light client is in-house

The beacon containers, `compute_domain`, `compute_signing_root` and the BLS checks mirror
ethlambda's `beacon::containers`, `beacon::signing` and `bls.rs`. They are spelled out here
instead of linked because ethlambda pins `libssz` to a git revision that differs from the
release ethrex builds on, which would put two copies of the SSZ library in one binary, and
because `ethlambda-crypto` carries the leanVM post-quantum stack. Neither client had the light
client containers or the sync protocol, which is about a thousand lines and is tested against
Sepolia data captured on 2026-10-19 (`tooling/partial/testdata/sepolia`): the bootstrap's
committee branch, two consecutive period updates, a finality update and an optimistic update,
with their BLS signatures verified against the committee the previous period's update
announced. The ethlambda fixture in `tooling/partial/testdata/ethlambda` pins the EIP-8440
proof shape both clients agree on.

### Refactors this points at

Each would shrink this crate and give the full node something too:

1. **Generic `eth_*` handlers in `ethrex-rpc`** over a trait that provides headers, accounts,
   storage, code and execution. The full node implements it with its store; this node with
   its pinned state; `rpc.rs` disappears.
2. **A single-key proof verifier in `ethrex-trie`**, the walk from `verify.rs`, next to
   `verify_range`. It is also what a full node needs to check an `eth_getProof` answer from
   someone else, and what the snap client should use when a peer picks the bounds.
3. **A beacon types crate shared with ethlambda** (containers, domains, signing roots, BLS),
   once the two clients build on the same `libssz`. The light client's `types.rs`, `merkle.rs`
   and `bls.rs` move there; `sync.rs` stays.

## Design notes

- **Not held.** Storage roots (stale under access lists), bodies and receipts (except the
  node's own transactions), logs, and state older than the eight retained blocks.
- **Limits.** `MAX_BAL_GAP = 32` blocks before reanchoring; `HEADER_WINDOW = 256` for
  `BLOCKHASH`; `RECENT_STATES = 8` retained stores; `MISS_ROUNDS = 64` fetch rounds per call;
  `WORKING_SET_CAP = 1024` keys.

## Live run

Checked on Sepolia on 2026-10-20 against the public ethpandaops Nimbus beacon API: the light
client bootstrapped from the node's finalized checkpoint, the node anchored and then followed
the chain one block per slot through block access lists, `eth_blockNumber` advanced from
11952372 to 11952409 over ten minutes, the watched account was proven at every head, and
`partial_status` reported the light client heads, finality and peer counts. Peers on Sepolia
that serve snap are few, so the first anchoring waits minutes for one.

`require` mode was run the same day against Sepolia's light client data with `--proofs.url`
pointing at a local stand-in that answered every head with a mock proof in the shape
ethlambda's `GET /eth/v1/beacon/execution_proofs/{block_id}` serves (no network produces
real chain proofs yet). The node anchored at the checkpoint's execution block, verified a proof
per slot with the mock engine, mapped each to its payload once the next header arrived, and
moved the anchor only onto proven payloads, 121 blocks in eight minutes with `anchorProven`
true throughout.

`require` mode was then run against a beacon node that really serves EIP-8440 proofs: an
ethlambda node from its `feat/beacon-execution-chain-proofs` branch on a local kurtosis devnet
(two Lighthouse nodes for light client data and finality, ethlambda for the proofs with its
validator client signing a mock chain proof per slot, ethrex as every execution layer, six
second slots). Over ten minutes from a checkpoint at slot 544 the node verified 94 proofs,
mapped 92 of them to payloads once the next header arrived, and moved its anchor from
execution block 409 to 563, always onto a proven payload and never more than one slot behind
the light client's optimistic head, with `anchorProven` true throughout and no warning but the
mock engine's. Two things that run taught are now in this page: a Lighthouse bootstrap exists
only for epoch-boundary checkpoints, and the prover's origin has to be at or before the
checkpoint.

## Testing

```sh
cargo test -p ethrex-partial --release
```

The suite builds a real Amsterdam chain with ethrex's own payload builder (`testing.rs`) and
answers snap requests from it, so proofs, access lists, receipts and the fetch-and-rerun loop
are exercised end to end with faults (dropped proof nodes, withheld leaves and access lists,
lying peers, reorgs, long gaps), and the light client and EIP-8440 modules against captured
Sepolia vectors and mock proofs.

## References

- [chibi](https://github.com/weiihann/chibi): the partial stateful node this follows.
- [EIP-8440: Execution Chain Proofs](https://github.com/ethereum/EIPs/pull/12464) and its
  [consensus specs](https://github.com/ethereum/consensus-specs/tree/ca292a5c5d42cf8f094aaf60778fc95a4db05d33/specs/_features/eip8440)
  (consensus-specs PR #5725, not merged).
- [EIP-8025: Optional Execution Proofs](https://eips.ethereum.org/EIPS/eip-8025), its
  [consensus specs](https://github.com/ethereum/consensus-specs/tree/master/specs/_features/eip8025),
  and the client work this node's proof plumbing follows: [Lighthouse's proof-engine sidecar client](https://github.com/sigp/lighthouse/tree/unstable/beacon_node/proof_engine),
  [eth-act Lighthouse `optional-proofs`](https://github.com/eth-act/lighthouse/tree/optional-proofs/beacon_node/proof_engine)
  and [Prysm #17690](https://github.com/OffchainLabs/prysm/pull/17690).
- [EIP-7928: Block-Level Access Lists](https://eips.ethereum.org/EIPS/eip-7928) and
  [snap/2 (EIP-8189) internals](../../internal/l1/snap_v2.md).
- [Gloas light client sync protocol](https://github.com/ethereum/consensus-specs/blob/ca292a5c5d42cf8f094aaf60778fc95a4db05d33/specs/gloas/light-client/sync-protocol.md).

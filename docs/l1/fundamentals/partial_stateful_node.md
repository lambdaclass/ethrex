# Partial stateful node

`ethrex partial-stateful-node` (alias `ethrex psn`) runs ethrex as a **partial stateful node**: a node that holds only the
accounts and storage its users read, proves every value it serves, and follows the chain
without executing blocks. It is built for wallets: a local JSON-RPC endpoint that is ready
in minutes, keeps kilobytes on disk, and trusts no RPC provider. The design follows
[chibi](https://github.com/weiihann/chibi), with [EIP-8440 execution chain proofs](https://github.com/ethereum/EIPs/pull/12464)
layered on top.

The mode is optional: build with `--features partial`. The implementation is the `ethrex-partial`
crate in `tooling/partial`.

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
1 to 3; upstream Lighthouse verifies through the external HTTP sidecar above. No client produces
or consumes EIP-8440 chain proofs: the recursive chain guest does not exist yet, so `observe`
and `require` can only be exercised against the mock engine until it does. Covering the gap
with one EIP-8025 proof per payload is possible (ethrex already has the `NewPayloadRequest`
SSZ container) but is not implemented.

## Design notes

- **Reused from ethrex.** The trie and its proof nodes, the EIP-7928 access list types and
  hash, header and receipt root computation, the devp2p stack (raw snap requests, eth/71
  and snap/2 access list fetches, header and body requests), levm through `VmDatabase`, and
  the RPC crate's request, response, error and JSON types.
- **ethlambda.** The beacon containers (`BeaconBlockHeader`, `SyncCommittee`, `SyncAggregate`),
  `compute_domain`, `compute_signing_root` and the BLS checks mirror ethlambda's
  `beacon::containers`, `beacon::signing` and `bls.rs`. They are spelled out here instead of
  linked because ethlambda pins `libssz` to a git revision that differs from the release
  ethrex builds on, which would put two copies of the SSZ library in one binary, and because
  `ethlambda-crypto` carries the leanVM post-quantum stack. Neither client has light client
  containers; extracting a shared crate is the natural follow-up.
- **In-house light client.** The sync protocol is about a thousand lines and is tested against
  Sepolia data captured on 2026-10-19 (`tooling/partial/testdata/sepolia`): the bootstrap's
  committee branch, two consecutive period updates, a finality update and an optimistic
  update, with their BLS signatures verified against the committee the previous period's
  update announced.
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
that serve snap are few, so the first anchoring can wait a minute for one.

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

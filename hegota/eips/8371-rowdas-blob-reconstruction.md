# EIP-8371: RowDAS - Distributed Blob Reconstruction
- **Layer:** CL (Networking)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acdc/184, 2026-08-06)
- **Authors:** Csaba Kiraly, Marco Munizaga
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8371) · [discussion](https://ethereum-magicians.org/t/eip-8371-rowdas-distributed-blobspace-reconstruction/29320)
- **Prior fork history:** none

## TL;DR
Under PeerDAS, rebuilding missing blob data falls on supernodes (nodes custodying all 128 columns), and every supernode redundantly reconstructs every missing row — load growing linearly with blob count. RowDAS adds 128 row subnets using cell-level partial messages so different node groups reconstruct different rows, letting ordinary nodes contribute and slashing supernode load.

## What it changes
- **New row subnets `data_row_{subnet_id}`**, `ROW_SUBNET_COUNT = 128` (fixed, independent of blob count so BPO forks don't reshuffle meshes). They carry *partial messages only* — no full-message fallback — and require the Gossipsub Partial Messages / Cell-Level Deltas extension (EIP-8136).
- **Blob→subnet mapping:** per-slot pseudo-random permutation via the consensus `compute_shuffled_index` (swap-or-not), seeded by `hash("ROW_SUBNET" || slot)` — stateless, evens load across slots, keeps distinct subnets per blob while blob count ≤ 128.
- **Node→subnet assignment:** one designated row subnet per node from `bytes [8:16]` of `hash(node_id)` (bytes `[0:8]` are already used for custody columns, so assignment is decorrelated for free). Public function of node ID → discovery needs no ENR change.
- **Phased reconstruction** (all gated on holding ≥64 verified cells of a row; timers start at first sight of the block root; work attaches to one block root per slot so equivocation can't amplify it):
  1. *Row reconstructors* (nodes with ≥64 column subnets, incl. high-custody stakers) MUST reconstruct rows on their designated subnet, after a small random delay.
  2. Supernodes SHOULD reconstruct all remaining rows after a longer delay, skipping rows observed complete — this matches current PeerDAS behavior, delayed.
  3. Any node on a row subnet SHOULD reconstruct still-incomplete rows after the longest delay — a supernode-independent recovery path with near-zero expected CPU.
- **Column topics:** extended with cell-level fanout — advertise/pull individual cells across subnets a node isn't subscribed to, optionally delayed to avoid competing with subscribed peers.
- **Bitmap signaling:** nodes send updated cell-availability bitmaps as cells arrive from any source (rows, columns, `getBlobs`); debouncing/rate-limiting recommended against quadratic messages. Peers may serve only half (64) of a row's cells — the rest is reconstructible.
- Explicitly **not** FullDAS: no sub-linear sampling, no column-wise encoding, no per-blob retrieval protocol (though row addressing is a prerequisite for rollups fetching only their own blobs — left to a future proposal).

## Motivation
Blob-count scaling makes per-supernode reconstruction CPU/bandwidth a contention point, and all supernodes duplicate the same work — a concentration/resilience problem. Row subnets pool custody across ~94 members each, so a subnet collectively covers >64 columns with high probability: a "virtual reconstructor" even where no member can reconstruct alone.

## Dependencies & related EIPs
- **Requires EIP-7594** (PeerDAS, Fusaka — the column/custody construct being extended) and **EIP-8136** (Cell-Level Deltas / Gossipsub partial messages — the wire mechanism row topics are built on; also a Hegota candidate).
- Unaffected by **EIP-7892** BPO forks by design (fixed subnet count).
- Synergizes with **EIP-8142** (Block-in-Blobs): BiB adds payload-blobs to every block, raising reconstruction volume that RowDAS distributes.
- Future per-blob retrieval for L2s would interact with **EIP-8383**-style retention windows (the EIP floats a row-serving retention window as a revision of its no-custody-obligation stance).

## Impact on ethrex / client teams
**None/minimal for ethrex (CL-only networking).** All changes live in libp2p gossipsub topics, custody/reconstruction logic, and CL peer scoring — ethrex is an EL client with no DAS stack. Indirect note: if ELs ever consume per-blob retrieval (the EIP's future extension), `engine_getBlobs`-adjacent flows could benefit, but that's out of scope here. **Effort for ethrex: none.**

## Open questions & controversies
- Row-subnet assignment is a public function of node ID → attackers can grind IDs to eclipse a chosen subnet; impact bounded to status-quo degradation, but it's a known weakness.
- Wire format (SSZ containers, bitmap encoding, GroupID derivation) is TBD in consensus-specs; random delay values and max-delay bounds are TBD — spec immaturity.
- Adoption gated on peers' libp2p stacks supporting Cell-Level Deltas; partial adoption means thinner subnets.
- Bitmap signaling risks quadratic message complexity without careful debouncing.
- A withholding producer can trigger reconstruction work on a doomed block (inherited from PeerDAS; RowDAS strictly reduces the wasted work but doesn't eliminate the vector).

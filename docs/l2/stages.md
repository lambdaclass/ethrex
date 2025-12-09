# Rollup stages and ethrex

This document relates the L2Beat rollup stage definitions to the current ethrex L2 stack as described in [their page](https://l2beat.com/stages). Stages are properties of a **deployed** L2, whereas Ethrex is a framework that different projects may configure and govern their own way. 

In this docs, when we talk about **Ethrex L2** we are referring to Ethrex in **Rollup mode**, not Validium, the main difference is that the former uses Ethereum L1 as the Data Availability layer whereas the latter doesn't.

Below are the answers to every question in each Stage.

## Stage 0

### Does the project call itself a rollup?

Yes, as pointed in [the introduction](./introduction.md), "Ethrex is a framework that lets you launch your own L2 rollup or blockchain".

As said before, whether a specific chain built with ethrex should call itself a rollup depends on how it is configured (e.g. rollups vs validium).

### Are L2 state roots posted on L1?

Yes.

Everytime that a batch is commited to the `OnChainProposer` in L1 the new state root of the L2 is sent and stored in a mapping that holds information of each batch, between those lies the `newStateRoot`.

### Does the project provide data availability on L1?

Yes.

When committing a batch in non-validium mode it will always require a blob hash different than zero, so a blob MUST be sent in the transaction to the `OnChainProposer` in the L1.

- The architecture docs state that the blob contains the **RLP‑encoded L2 blocks and fee configuration**:
  - See [“Reconstructing state or Data Availability”](./fundamentals/data_availability.md#reconstructing-state-or-data-availability).
  - See [“Transition to RLP‑encoded Blocks”](./architecture/overview.md#transition-to-rlp-encoded-blocks) and [“L1 contract checks”](./architecture/overview.md#l1-contract-checks).
- The blob commitment (`blobKZGVersionedHash` / `blobVersionedHash`) is included in the batch commitment and re‑checked during proof verification.

This means that, in rollup mode, all data needed to reconstruct the L2 (transactions and state) is published on L1 as blobs.

### Is software capable of reconstructing the rollup’s state available?

Yes.

- The L2 node can follow the L1 commitments and blobs to reconstruct the L2 state.
- The state‑reconstruction path is actively tested:
  - `crates/l2/tests/state_reconstruct.rs` replays a fixed set of blobs to reconstruct a known final state.
  - [State reconstruction blobs](../developers/l2/state-reconstruction-blobs.md) documents how to generate and use blobs for this test.
- The [“Reconstructing state or Data Availability”](./fundamentals/data_availability.md#reconstructing-state-or-data-availability) and [“EIP‑4844 (a.k.a. Blobs)”](./fundamentals/data_availability.md#eip-4844-aka-blobs) sections and the [prover docs](../prover/prover.md) describe how the published data is used to reconstruct and verify state.

### Does the project use a proper proof system?

Ethrex is designed as a **zk rollup / zk‑validium framework** and includes a full validity‑proof pipeline:

- The prover (`ethrex-prover`, documented in the [prover docs](../prover/prover.md)) uses a zkVM to re‑execute batches using an execution witness and commits:
  - Initial and final state roots.
  - Withdrawal log Merkle root.
  - Privileged transaction rolling hash.
  - Data‑availability commitments (blob hashes / state‑diff commitments, depending on the version).
- The L1 `OnChainProposer` contract verifies the proofs on‑chain:
  - Directly, via Groth16 verifiers for Risc0/SP1/TDX (`IRiscZeroVerifier`, `ISP1Verifier`, `ITDXVerifier`).
  - Or indirectly, via an Aligned Layer aggregator (`verifyBatchesAligned` path).
- The public inputs are checked against the committed batch data in `_verifyPublicData`, and a failed check or proof verification makes the transaction revert.

However, proof *enforcement* is a deployment choice:

- `OnChainProposer` has boolean flags like `REQUIRE_RISC0_PROOF`, `REQUIRE_SP1_PROOF`, `REQUIRE_TDX_PROOF`.
- A chain that disables all proofs for production would **not** meet the “proper proof system is used to accept state roots” requirement, even though ethrex supports it.

### Are there at least 5 external actors that can submit a fraud proof?

Not applicable as stated, and not implemented as a fraud‑proof system.

- Ethrex uses **validity proofs**, not fraud proofs. There is no on‑chain “challenge game” where watchers can submit alternate traces to invalidate a state root.
- The standard `OnChainProposer` variant (`crates/l2/contracts/src/l1/OnChainProposer.sol`) restricts `commitBatch`/`verifyBatch` to an allowlisted set of sequencer addresses (`onlySequencer`), so even proof submission is not permissionless there.
- The “based” variant (`crates/l2/contracts/src/l1/based/OnChainProposer.sol`) keeps `commitBatch` restricted to the leader sequencer but makes `verifyBatch` and `verifyBatchesAligned` externally callable, so anyone can *submit* a validity proof; it is still not a fraud‑proof scheme.

So there is no fraud‑proof system with ≥5 external challengers today. Ethrex targets the zk‑rollup path in the stage specification, not the optimistic‑rollup / fraud‑proof path.

## Stage 1 questions

Stage 1 is mostly about **governance and upgrades**: who can block/alter L2→L1 messages, under what assumptions, and with what exit windows.

### Is the Security Council properly set up?

No Security Council is encoded in the contracts; governance is left to deployments.

- The main L1 contracts (`CommonBridge`, `OnChainProposer`, `Router`) and L2 system contracts are **UUPS proxies** controlled by a single `owner` (see [contracts fundamentals](./fundamentals/contracts.md)).
- Ownership uses `Ownable2StepUpgradeable`, but there is no built‑in notion of:
  - “Security Council” with ≥8 members.
  - A 75% signer threshold at the contract level.
  - Entity‑level de‑duplication or geographical / organizational diversity.
- The docs describe generic upgrade and ownership‑transfer procedures, not a specific governance setup or published council identities.

In practice, a project deploying an ethrex‑based chain can point the `owner` of these proxies to a multi‑sig that implements the Stage‑1 Security Council semantics, but that happens outside this codebase.

### Can only compromise of ≥75% of the Security Council censor or forge L2→L1 messages?

Out of the box, no; the contracts give broader powers to the owner and to sequencers.

Some relevant facts from the current implementation:

- `CommonBridge` can be **paused** by `owner`, which immediately blocks withdrawals and generic L2→L1 messages, indefinitely if desired.
- `CommonBridge` exposes `upgradeL2Contract`, which lets the owner upgrade L2 system contracts (including the bridge and messenger) without any enforced delay, via privileged transactions.
- `OnChainProposer` and `CommonBridge` are upgradeable; a new implementation can arbitrarily change how commitments, proofs, and withdrawal Merkle roots are handled.
- Sequencers (or in the based model, the leader sequencer) control which batches are committed and when; censorship at the sequencing layer is mitigated by design (escape‑hatch ideas are mentioned but not fully specified yet in [“What the sequencer can do”](./architecture/overview.md#what-the-sequencer-can-do)), but not eliminated.

There is no protocol‑level guarantee that “the only way to block a withdrawal indefinitely or to push an invalid withdrawal is compromising ≥75% of a Security Council.” Achieving that property would require:

- Putting all upgrade and pause powers behind a suitably structured council / multi‑sig.
- Adding on‑chain constraints (e.g. timelocks, limited emergency powers) that the current contracts do not enforce by themselves.

### 7‑day challenge / exit windows

Ethrex’s contracts do not implement:

- A mandatory **7‑day (or longer) challenge period** for proving batches.
- A guaranteed **exit window** before upgrades take effect.

Batch verification is immediate once a valid proof is provided and accepted, and upgrades are governed only by the UUPS owner. A deployment could wrap these contracts with timelocks or additional governance contracts to approximate Stage‑1 behaviour, but that is not present in this repository.

## Stage 2 questions

Stage 2 focuses on **fully permissionless proving / challenging** and on tightly constraining emergency upgrade powers.

### Is the fraud / validity proof system permissionless?

It depends on which `OnChainProposer` variant is deployed:

- **Standard OnChainProposer** (`crates/l2/contracts/src/l1/OnChainProposer.sol`):
  - `commitBatch` and `verifyBatch` are restricted to `authorizedSequencerAddresses` via `onlySequencer`.
  - Submitting proofs is *not* permissionless; it is tied to the sequencer set.
- **Based OnChainProposer** (`crates/l2/contracts/src/l1/based/OnChainProposer.sol`):
  - `commitBatch` is restricted to the leader sequencer (`onlyLeaderSequencer`), but
  - `verifyBatch` and `verifyBatchesAligned` are externally callable without role checks.
  - Anyone can submit a validity proof for a committed batch and cause it to be verified, provided the proof is correct.

So:

- The framework supports a **permissionless validity‑proof submission** model in the based variant.
- The non‑based variant uses an allowlisted sequencer set and does **not** meet the Stage‑2 “permissionless” requirement.

In both cases, this is a validity‑proof system, not a fraud‑proof system.

### Do users have at least 30 days to exit in case of unwanted upgrades?

No, not at the protocol level.

- UUPS proxies for `CommonBridge`, `OnChainProposer`, and other core contracts can be upgraded by the `owner` **without any on‑chain delay or exit window**.
- There is no built‑in mechanism that:
  - Announces upgrades in advance.
  - Enforces a 30‑day grace period where withdrawals remain possible under the old rules.
  - Forces upgrades initiated by non‑council actors to include an exit window.

Any such guarantees would need to be provided by off‑chain governance processes or additional contracts not present here.

### Is the Security Council restricted to act only due to errors detected on‑chain?

No, because there is no dedicated Security Council and no restriction on how upgrade powers are used.

- `CommonBridge`, `OnChainProposer`, `Router`, and related contracts expose:
  - `pause` / `unpause` functionality.
  - Upgrade hooks (`_authorizeUpgrade`) guarded only by `onlyOwner`.
  - In `CommonBridge`, the ability to upgrade L2 system contracts via privileged messages (`upgradeL2Contract`).
- These powers are *general*: they can be used for bug fixes, parameter changes, or arbitrary logic changes, and are not gated by any on‑chain bug detector.

To meet the Stage‑2 requirement, an ethrex‑based deployment would need additional constraints such as:

- Limiting the scope of emergency functions to well‑defined on‑chain conditions (e.g. contradictory proofs).
- Separating routine governance from hardened emergency‑only keys.

## Summary

- Ethrex provides the **core technical ingredients for a zk rollup**: state commitments on L1, DA via blobs in rollup mode, open‑source node software capable of reconstructing state, and a validity‑proof system that ties execution, messaging, and DA together.
- The framework **does not prescribe or implement** the Security Council, timelocks, or exit‑window mechanisms that dominate the Stage‑1 and Stage‑2 definitions; those are left to each deployment’s governance and contract wiring.
- An individual ethrex‑based chain can likely achieve Stage‑0 (zk‑rollup path) by:
  - Running in rollup mode with DA on L1.
  - Enforcing validity proofs on L1 in production.
  - Publishing and supporting reconstruction tooling.
- Reaching Stage‑1 or Stage‑2 would require **additional contracts and governance layers** around the primitives defined in this repository.

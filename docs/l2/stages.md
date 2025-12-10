# Rollup stages and ethrex

This document relates the L2Beat rollup stage definitions to the current ethrex L2 stack as described in [their page](https://l2beat.com/stages). Stages are properties of a **deployed** L2, whereas Ethrex is a framework that different projects may configure and govern their own way. 

In this docs, when we talk about **Ethrex L2** we are referring to Ethrex in **Rollup mode**, not Validium, the main difference is that the former uses Ethereum L1 as the Data Availability layer whereas the latter doesn't.

Below are the answers to every question or requirement of each Stage.

## Stage 0

### Does the project call itself a rollup?

Yes. 

As pointed in [the introduction](./introduction.md)
> Ethrex is a framework that lets you launch your own L2 rollup or blockchain.

### Are L2 state roots posted on L1?

Yes.

Everytime that a batch is commited to the `OnChainProposer` in L1 the new state root of the L2 is sent and stored in a mapping that holds information of each batch, between those lies the `newStateRoot`.

### Does the project provide data availability on L1?

Yes.

When committing a batch in non-validium mode it will always require a blob hash different than zero, so a blob MUST be sent in the transaction to the `OnChainProposer` in the L1.

- The [architecture docs](./architecture/overview.md) state that the blob contains the **RLP‑encoded L2 blocks and fee configuration**:
- The blob commitment (`blobKZGVersionedHash` / `blobVersionedHash`) is included in the batch commitment and re‑checked during proof verification.

This means that all data needed to reconstruct the L2 (transactions and state) is published on L1 as blobs.

### Is software capable of reconstructing the rollup’s state available?

Yes.

- The L2 node can follow the L1 commitments and blobs to reconstruct the L2 state.
- The state‑reconstruction path is actively tested:
  - `crates/l2/tests/state_reconstruct.rs` replays a fixed set of blobs to reconstruct a known final state.
  - [State reconstruction blobs](../developers/l2/state-reconstruction-blobs.md) documents how to generate and use blobs for this test.
- The [“Reconstructing state or Data Availability”](./fundamentals/data_availability.md#reconstructing-state-or-data-availability) and [“EIP‑4844 (a.k.a. Blobs)”](./fundamentals/data_availability.md#eip-4844-aka-blobs) sections and the [prover docs](../prover/prover.md) describe how the published data is used to reconstruct and verify state.

### Does the project use a proper proof system?

Yes, assuming proofs are enabled.

Ethrex supports multiple proving systems, such as SP1 and RISC0.
The `OnChainProposer` contract can be configured to require any combination of these proofs when verifying. A batch is only verified on L1 if all configured proofs pass and their public inputs match the committed data (state roots, withdrawals, blobs, etc.).

### Are there at least 5 external actors that can submit a fraud proof?

Not applicable, and not implemented as a fraud‑proof system.

Ethrex uses **validity proofs**, not fraud proofs. There is no on‑chain “challenge game” where watchers can submit alternate traces to invalidate a state root.

## Stage 1

The main requirement for Ethrex L2 to belong to stage 1 is:

Compromising ≥75% of the Security Council should be the only way (other than bugs) for the rollup to indefinitely block an L2→L1 message (e.g. a withdrawal) or push an invalid L2→L1 message (e.g. an invalid withdrawal) with an exit window shorter than 7 days. Any other mechanism that can affect such messages must give users at least a 7‑day exit window.

In the current ethrex contracts:

- Both `OnChainProposer` and `CommonBridge` are **UUPS upgradeable** and **Ownable2Step**. Upgrades are authorized by a single `owner` address (which can itself be a multisig, but that is outside this repo).
- The contracts expose `pause` / `unpause` and `_authorizeUpgrade` gated by `onlyOwner`, with **no built‑in timelock, exit window, or dedicated Security Council logic**.
- Nothing in the contracts enforces a specific number of council members, a 75% threshold, or entity‑level decentralization; this must be provided by how the `owner` is chosen (e.g. a Safe or custom governance system).

Out of the box, ethrex therefore **does not satisfy Stage 1’s Security Council and exit‑window requirements**. A network built on ethrex could meet them by:

- Pointing the proxy `owner` to an appropriately configured multisig / governance system, and
- Introducing timelocks / exit windows around contract upgrades at the governance layer.

## Stage 2

Stage 2 focuses on **fully permissionless proving / challenging** and on tightly constraining emergency upgrade powers.

### Is the validity proof system permissionless?

No.

In the **Standard OnChainProposer** (`crates/l2/contracts/src/l1/OnChainProposer.sol`) committing and verifying batches are restricted to the authorized sequenced addresses only. So submitting proofs is not permissionless.

### Do users have at least 30 days to exit in case of unwanted upgrades?

No.

There is **no protocol‑level exit window** tied to contract upgrades; UUPS upgrades can be executed by the `owner` without a mandatory delay.

### Is the Security Council restricted to act only due to errors detected on‑chain?

No.

There is **no built‑in Security Council role** that is restricted to on‑chain bug responses; the `owner` can pause or upgrade contracts for any reason allowed by the implementation.

## Summary

Ethrex L2 currently satisfies all Stage 0 requirements, in order to become a Stage 1 rollup it should at least incorporate the notion of a Security Council and an exit window mechanism in case of contract upgrades.

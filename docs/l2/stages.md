# Rollup stages and Ethrex

This document relates the [L2Beat rollup stage definitions](https://l2beat.com/stages) to the current Ethrex L2 stack. Stages are properties of a **deployed** L2, whereas Ethrex is a framework that different projects may configure and govern in their own way. We are going to assume that if Ethrex provides the functionality to deploy a Stage X rollup, then it's enough to consider Ethrex to be in that Stage. However, note that one could deploy an L2 with Ethrex while choosing not to enable certain available properties, and therefore end up at a lower stage than the maximum potential (e.g. choosing not to have a Security Council).

In this document, when we talk about **Ethrex L2** we are referring to Ethrex in **Rollup mode**, not Validium. The main difference is that the former uses Ethereum L1 as the Data Availability layer whereas the latter does not.

Below are the answers to every question or requirement of each Stage.

## Stage 0

### Summary

| Requirement | Status | Reason (short) |
| --- | --- | --- |
| Project calls itself a rollup | ✅ | Docs describe Ethrex as a framework to launch an L2 rollup. |
| L2 state roots posted on L1 | ✅ | Each committed batch stores `newStateRoot` in `OnChainProposer` on L1. |
| Data availability on L1 | ✅ | In rollup mode every batch must publish a non‑zero EIP‑4844 blob hash. |
| Software to reconstruct state | ✅ | Node, blobs tooling, and prover docs describe how to replay blobs and rebuild state. |
| Proper proof system used | ✅ | Batches are verified using validity proofs (SP1/RISC0/TDX/Aligned) that must match committed data. |

### Does the project call itself a rollup?

Yes. 

As pointed out in [the introduction](./introduction.md)
> Ethrex is a framework that lets you launch your own L2 rollup or blockchain.

### Are L2 state roots posted on L1?

Yes.

Every time that a batch is committed to the `OnChainProposer` in L1, the new state root of the L2 is sent and stored in a mapping that holds information about each batch; among that information is the `newStateRoot`.

### Does the project provide data availability on L1?

Yes.

When committing a batch in non‑validium mode it always requires a non‑zero blob hash, so a blob MUST be sent in the transaction to the `OnChainProposer` on L1.

- The [architecture docs](./architecture/overview.md) state that the blob contains the **RLP‑encoded L2 blocks and fee configuration**.
- The blob commitment (`blobKZGVersionedHash` / `blobVersionedHash`) is included in the batch commitment and re‑checked during proof verification.

This means that all data needed to reconstruct the L2 (transactions and state) is published on L1 as blobs.

### Is software capable of reconstructing the rollup’s state available?

Yes.

- The L2 node can follow the L1 commitments and blobs to reconstruct the L2 state.
- The [State reconstruction blobs](../developers/l2/state-reconstruction-blobs.md) doc explains how to generate and use blobs for a test that replays a fixed set of blobs to reconstruct a known final state.
- The [“Reconstructing state or Data Availability”](./fundamentals/data_availability.md#reconstructing-state-or-data-availability) and [“EIP‑4844 (a.k.a. Blobs)”](./fundamentals/data_availability.md#eip-4844-aka-blobs) sections and the [prover docs](../prover/prover.md) describe how the published data is used to reconstruct and verify state.

### Does the project use a proper proof system?

Yes, assuming proofs are enabled.

Ethrex supports multiple proving systems, such as SP1 and RISC0.
The `OnChainProposer` contract can be configured to require any combination of these proofs when verifying. A batch is only verified on L1 if all configured proofs pass and their public inputs match the committed data (state roots, withdrawals, blobs, etc.).

### Are there at least 5 external actors that can submit a fraud proof?

Not applicable, and not implemented as a fraud‑proof system.

Ethrex uses **validity proofs**, not fraud proofs. There is no on‑chain “challenge game” where watchers can submit alternate traces to invalidate a state root.

## Stage 1

### Summary

| Requirement | Status | Reason (short) |
| --- | --- | --- |
| Censorship‑resistant L2→L1 messages | ❌ | Sequencer can indefinitely censor withdrawals; there is no forced‑inclusion mechanism on L1. |
| Sequencer cannot push invalid L2→L1 messages | ✅ | Sequencer alone cannot make L1 accept an invalid withdrawal; this would require a contract change and VK update controlled by the Security Council (`owner`). |
| ≥7‑day exit window for non‑SC upgrades | ✅ | Only the Security Council (`owner`) can upgrade contracts; there is no upgrade path from entities outside the Council. |

Ethrex L2 does not fully satisfy the requirements to be Stage 1 because the sequencer can indefinitely block an L2→L1 message (withdrawal), so it is not censorship‑resistant. This could be avoided by implementing forced inclusion of withdrawals enforced by the contracts on L1, in which the user of the L2 can send their withdrawal to the contract on L1 and the sequencer must include it in a subsequent batch in a limited time frame; otherwise they cannot continue sequencing.

### The only way (other than bugs) for a rollup to indefinitely block an L2→L1 message (e.g. a withdrawal) or push an invalid L2→L1 message (e.g. an invalid withdrawal) is by compromising ≥75% of the Security Council.

Both `OnChainProposer` and `CommonBridge` are upgradeable contracts, and these are authorized by a single `owner` address. Ethrex itself does not explicitly define a Security Council, but it could have one if the owner were a multisig composed of all members. According to L2Beat requirements this Council should have at least 8 members. Note that if the owner is treated as a Security Council there would be no other actors with more power than this one.

- The sequencer could indefinitely block/censor an L2→L1 message by simply not including the withdrawal transaction in an L2 block.
- The sequencer cannot unilaterally make L1 accept an invalid L2→L1 message; this would require a change in the code, which would then require updating the Verifying Key in the `OnChainProposer`, and only the Security Council (owner) is capable of updating the VK.


### Upgrades initiated by entities outside of the Security Council are allowed if they provide at least a 7‑day exit window.

In Ethrex L2 contracts there is no concept of an exit window, but there are also no entities other than the Security Council that can update them. So these kinds of upgrades are not allowed.

## Stage 2

Stage 2 focuses on **fully permissionless proving / challenging** and on tightly constraining emergency upgrade powers.

### Summary

| Requirement | Status | Reason (short) |
| --- | --- | --- |
| Permissionless validity proof system | ❌ | In the standard `OnChainProposer` only authorized sequencer addresses can commit and verify batches. |
| ≥30‑day exit window for unwanted upgrades | ❌ | There is no protocol‑level exit window; UUPS upgrades by `owner` have no mandatory delay. |
| Security Council restricted to on‑chain errors | ❌ | The `owner` can pause or upgrade contracts for any reason allowed by the implementation. |

### Is the validity proof system permissionless?

No.

In the **Standard OnChainProposer** (`crates/l2/contracts/src/l1/OnChainProposer.sol`) committing and verifying batches are restricted to the authorized sequencer addresses only, so submitting proofs is not permissionless.

### Do users have at least 30 days to exit in case of unwanted upgrades?

No.

There is **no protocol‑level exit window** tied to contract upgrades; UUPS upgrades can be executed by the `owner` without a mandatory delay.

### Is the Security Council restricted to act only due to errors detected on‑chain?

No.

There is **no built‑in Security Council role** that is restricted to on‑chain bug responses; the `owner` can pause or upgrade contracts for any reason allowed by the implementation.

## Conclusion

Ethrex L2 currently satisfies all Stage 0 requirements and is very close to becoming a Stage 1 rollup. It just needs to be censorship‑resistant regarding L2→L1 messages (e.g. withdrawals), because currently the sequencer could ignore withdrawal transactions.

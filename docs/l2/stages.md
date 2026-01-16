# Rollup Stages and ethrex

This document explains how the [L2Beat rollup stage definitions](https://l2beat.com/stages) map to the current ethrex L2 stack.

## Important Distinctions

Stages are properties of a **deployed** L2, whereas ethrex is a framework that different projects may configure and govern in their own way. In what follows we make two simplifying assumptions:

- If ethrex provides the functionality required to deploy a Stage X rollup, we consider ethrex capable of achieving Stage X, even if a particular deployment chooses not to enable some features.
- When we talk about **ethrex L2** we are referring to ethrex in **rollup mode**, not Validium. In rollup mode, Ethereum L1 is the data availability layer; in Validium mode it is not.

The L2Beat framework evaluates *decentralization* specifically, not security from bugs. A Stage 2 rollup could still have vulnerabilities if the proof system is experimental or unaudited.

## Stage 0

Stage 0 ("Full Training Wheels") represents the basic operational requirements for a rollup.

### Summary

| Requirement | Status | Details |
|-------------|--------|---------|
| Project calls itself a rollup | ✅ Met | Docs describe ethrex as a framework to launch an L2 rollup |
| L2 state roots posted on L1 | ✅ Met | Each committed batch stores `newStateRoot` in `OnChainProposer` |
| Data availability on L1 | ✅ Met | In rollup mode every batch must publish a non-zero EIP-4844 blob hash |
| Software to reconstruct state | ✅ Met | Node, blobs tooling, and prover docs describe how to rebuild state |
| Proper proof system used | ✅ Met | Batches verified using zkVM validity proofs (SP1/RISC0) or TDX attestations |

### Detailed Analysis

#### Does the project call itself a rollup?

Yes. As stated in [the introduction](./introduction.md):

> Ethrex is a framework that lets you launch your own L2 rollup or blockchain.

#### Are L2 state roots posted on L1?

Yes. Every time a batch is committed to the `OnChainProposer` on L1, the new L2 state root is sent and stored in the `batchCommitments` mapping as `newStateRoot` for that batch.

#### Does the project provide data availability on L1?

Yes. When committing a batch in rollup mode (non-validium), the transaction must include a non-zero blob hash, so a blob MUST be sent to the `OnChainProposer` on L1.

- The [architecture docs](./architecture/overview.md) state that the blob contains the **RLP-encoded L2 blocks and fee configuration**
- The blob commitment (`blobKZGVersionedHash`) is included in the batch commitment and re-checked during proof verification

This means all data needed to reconstruct the L2 (transactions and state) is published on L1 as blobs.

#### Is software capable of reconstructing the rollup's state available?

Yes.

- The L2 node provides the `ethrex l2 reconstruct` subcommand to follow L1 commitments and reconstruct the state from blobs
- The [state reconstruction blobs](../developers/l2/state-reconstruction-blobs.md) doc explains how to generate and use blobs for replaying state
- The [data availability](./fundamentals/data_availability.md) and [prover docs](../prover/prover.md) describe how published data is used to reconstruct and verify state

#### Does the project use a proper proof system?

Yes, assuming proofs are enabled.

ethrex supports multiple proving mechanisms:
- **zkVM validity proofs**: SP1 and RISC0
- **TDX attestations**: TEE-based verification
- **Aligned Layer**: Optional proof aggregation for cost efficiency

The `OnChainProposer` contract can be configured to require many combinations of these mechanisms. A batch is only verified on L1 if all configured proofs pass and their public inputs match the committed data (state roots, withdrawals, blobs, etc.).

#### Are there at least 5 external actors that can submit a fraud proof?

Not applicable. ethrex uses **validity proofs**, not fraud proofs. There is no on-chain "challenge game" where watchers submit alternate traces to invalidate a state root.

### Stage 0 Assessment

**ethrex L2 meets all Stage 0 requirements.**

## Stage 1

Stage 1 ("Limited Training Wheels") requires that users have trustless exit guarantees, with a Security Council retaining only limited emergency powers.

### Core Principle

> "The only way (other than bugs) for a rollup to indefinitely block an L2→L1 message or push an invalid L2→L1 message is by compromising ≥75% of the Security Council."

### Summary

| Requirement | Status | Details |
|-------------|--------|---------|
| Censorship-resistant L2→L1 messages | ❌ Gap | Sequencer can indefinitely censor withdrawals; no forced-inclusion mechanism |
| Sequencer cannot push invalid messages | ✅ Met | Invalid withdrawals require contract/VK changes controlled by owner |
| ≥7-day exit window for non-SC upgrades | ✅ Met | Only owner can upgrade; no non-SC upgrade path exists |

### Detailed Analysis

#### Can L2→L1 messages be censored?

**Yes, this is the main Stage 1 gap.**

The sequencer can indefinitely block/censor an L2→L1 message (e.g., a withdrawal) by simply not including the withdrawal transaction in an L2 block. This does not require compromising the owner/Security Council.

**What's missing**: A forced-inclusion mechanism where users can submit their withdrawal directly on L1, and the sequencer must include it in a subsequent batch within a bounded time window or lose sequencing rights.

> [!NOTE]
> This is the primary blocker for Stage 1 compliance. Implementing forced inclusion of withdrawals enforced by L1 contracts would address this gap.

#### Can the sequencer push invalid L2→L1 messages?

**No.** The sequencer cannot unilaterally make L1 accept an invalid L2→L1 message. This would require:
- Changing contract code
- Updating the verifying key in `OnChainProposer`

Only the Security Council (owner) can perform those upgrades.

#### What about upgrades from entities outside the Security Council?

In ethrex L2 contracts, there are no entities other than the owner that can perform upgrades. Therefore:
- Upgrades initiated by entities outside the Security Council are not possible
- If such an upgrade path were introduced, it would need to provide the required 7-day exit window

#### Security Council Configuration

Both `OnChainProposer` and `CommonBridge` are upgradeable contracts controlled by a single `owner` address. ethrex itself does not hard-code a Security Council, but a deployment can introduce one by making the `owner` a multisig.

According to L2Beat requirements, the Security Council should have:
- At least 8 members
- ≥75% threshold for critical actions
- Diverse signers from different organizations/jurisdictions

### Stage 1 Assessment

**ethrex L2 does not meet Stage 1 requirements today.**

The main gap is censorship-resistant L2→L1 messages. The sequencer can ignore withdrawal transactions indefinitely, and there is no forced-inclusion mechanism (unlike the existing forced-inclusion mechanism for deposits).

### Path to Stage 1

To achieve Stage 1, ethrex would need:

1. **Forced withdrawal inclusion**: Implement an L1 mechanism where users can submit withdrawals directly, with sequencer penalties for non-inclusion
2. **Security Council multisig**: Deploy owner as an 8+ member multisig with ≥75% threshold
3. **Exit window enforcement**: ethrex has Timelock functionality that gates the `OnChainProposer`, but deployment configuration must enforce ≥7 day delays for non-emergency upgrades

## Stage 2

Stage 2 ("No Training Wheels") requires fully permissionless proving and tightly constrained emergency upgrade powers.

### Summary

| Requirement | Status | Details |
|-------------|--------|---------|
| Permissionless validity proofs | ❌ Gap | Only authorized sequencers can commit and verify batches |
| ≥30-day exit window | ❌ Gap | No protocol-level exit window; UUPS upgrades have no mandatory delay |
| SC restricted to on-chain errors | ❌ Gap | Owner can pause/upgrade for any reason |

### Detailed Analysis

#### Is the validity proof system permissionless?

**No.** In the standard `OnChainProposer` implementation (`crates/l2/contracts/src/l1/OnChainProposer.sol`), committing and verifying batches are restricted to authorized sequencer addresses only. Submitting proofs is not permissionless.

#### Do users have at least 30 days to exit before unwanted upgrades?

**No.** There is no protocol-level exit window tied to contract upgrades. UUPS upgrades can be executed by the owner without a mandatory delay.

#### Is the Security Council restricted to act only due to on-chain errors?

**No.** There is no built-in restriction that limits the owner to responding only to detected on-chain bugs. The owner can pause or upgrade contracts for any reason.

### Stage 2 Assessment

**ethrex L2 does not meet Stage 2 requirements.**

### Path to Stage 2

To achieve Stage 2, ethrex would need (in addition to Stage 1 requirements):

1. **Permissionless proving**: Allow anyone to submit validity proofs for batches
2. **30-day exit window**: Implement mandatory delay for all contract upgrades
3. **Restricted SC powers**: Limit Security Council actions to adjudicable on-chain bugs only
4. **Mature proof system**: Battle-tested ZK provers with comprehensive security audits

## Comparison with Other Rollups

### Based Rollups

Based rollups delegate sequencing to Ethereum L1 validators rather than using a centralized sequencer. This is particularly relevant for ethrex as it implements based sequencing (currently in development).

| Project | Current Stage | Main Gaps | Proof System | Sequencer Model |
|---------|---------------|-----------|--------------|-----------------|
| **ethrex L2** | Stage 0 | Forced inclusion, permissionless proving | Multi-proof (ZK + TEE) | Based (round-robin) |
| Taiko Alethia | Stage 0* | ZK not mandatory, upgrade delays | Multi-proof (SGX mandatory, ZK optional) | Based (permissionless) |
| Surge | Not deployed | N/A (template) | Based on Taiko stack | Based (L1 validators) |

**Taiko Alethia** is the first based rollup on mainnet. It requires two proofs per block: SGX (Geth) is mandatory, plus one of SGX (Reth), SP1, or RISC0. **Critically, blocks can be proven with TEE only (no ZK)** if both SGX verifiers are used. As of early 2025, only ~30% of blocks use ZK proofs. L2BEAT warns that "funds can be stolen if a malicious block is proven by compromised SGX instances." Taiko plans to require 100% ZK coverage with the Shasta fork in Q4 2025.

> *L2BEAT currently classifies Taiko as "not even Stage 0" because "the proof system is still under development." However, Taiko has been a multi-prover based rollup since the Pacaya fork and the system is architecturally prepared for Stage 0. This appears to be a classification nuance rather than a fundamental gap.

**Surge** is a based rollup template by Nethermind, built on the Taiko stack and designed to target Stage 2 from inception. It removes centralized sequencing entirely, letting Ethereum validators handle transaction ordering. Not yet deployed as a production rollup.

### ZK Rollups

| Project | Current Stage | Main Gaps | Proof System |
|---------|---------------|-----------|--------------|
| **ethrex L2** | Stage 0 | Forced inclusion, permissionless proving | Multi-proof (ZK + TEE) |
| Scroll | Stage 1 | 30-day window, multi-prover | ZK validity proofs |
| zkSync Era | Stage 0* | Evaluation pending, forced inclusion | ZK validity proofs |
| Starknet | Stage 1 | 30-day window, SC restrictions | ZK validity proofs (STARK) |

**Scroll** became the first ZK rollup to achieve Stage 1 (April 2025) through the Euclid upgrade, which introduced permissionless sequencing fallback and a 12-member Security Council with 75% threshold.

**zkSync Era** is currently experiencing a **proof system pause** due to a vulnerability, causing partial liveness failure. Previously, a critical bug in zk-circuits was discovered that could have led to $1.9B in potential losses if exploited.

> *L2BEAT states they "haven't finished evaluation" of zkSync Era's Stage 1 elements - not that zkSync fails requirements. The main pending item is a forced inclusion mechanism. With 75% of proving already delegated to external provers and decentralized sequencing (ChonkyBFT) underway, zkSync appears architecturally Stage 1-ready.

**Starknet** reached Stage 1 but shares its SHARP verifier with other StarkEx rollups. The verifier can be changed by a 2/4 multisig with 8-day delay. The Security Council (9/12) retains instant upgrade capability. This shared verifier creates concentration risk across multiple chains.

### Optimistic Rollups

| Project | Current Stage | Main Gaps | Proof System |
|---------|---------------|-----------|--------------|
| Arbitrum One | Stage 1 | SC override power, 30-day window | Optimistic (fraud proofs) |
| Optimism | Stage 1 | Exit window, SC restrictions | Optimistic (fault proofs) |

**Arbitrum One** uses BoLD (Bounded Liquidity Delay) for permissionless fraud proofs - anyone can challenge state assertions. However, Arbitrum remains Stage 1 because the Security Council retains broad override powers. Stage 2 requires restricting SC to "provable bugs only" and extending exit windows to 30 days. The ~6.4 day withdrawal delay is inherent to the optimistic model.

**Optimism** has permissionless fault proofs but L2BEAT notes: "There is no exit window for users to exit in case of unwanted regular upgrades as they are initiated by the Security Council with instant upgrade power." Both Arbitrum and Optimism are **technically ready for Stage 2** but held back by **intentional governance constraints**, not technical limitations.

### L2BEAT Risk Summary

| Project | Critical Warnings |
|---------|-------------------|
| Taiko Alethia | Funds at risk from compromised SGX; ZK optional; unverified contracts |
| Scroll | No upgrade delay; emergency verifier upgrade occurred Aug 2025 |
| zkSync Era | Proof system currently paused; prior $1.9B bug discovered |
| Starknet | Shared SHARP verifier; SC has instant upgrade power |
| Arbitrum One | Malicious upgrade risk; optimistic delay (~6.4 days) |
| Optimism | No exit window for SC upgrades; dispute game vulnerabilities |

> [!WARNING]
> All rollups carry risks. Even Stage 1 rollups retain Security Council powers that could theoretically be abused. Stage 2 remains unachieved by any production rollup as of early 2025.

### Key Observations

1. **No rollup has achieved Stage 2 yet** - All production rollups remain at Stage 0 or Stage 1
2. **Classification vs architecture gaps** - Some rollups (Taiko, zkSync Era) are classified lower than their architecture supports due to L2BEAT evaluation timing or minor gaps
3. **Governance is the bottleneck** - Arbitrum and Optimism have permissionless proofs but are held at Stage 1 by intentional Security Council powers, not technical limitations
4. **Based rollups are newer** - Taiko and ethrex are pioneering based sequencing, both at Stage 0
5. **Multi-proof is emerging** - ethrex, Taiko, and Scroll are all exploring multi-proof systems for enhanced security

## Recommendations

### For Stage 1 Compliance

1. **Implement forced withdrawal inclusion**
   - Users can submit withdrawal requests directly to L1
   - Sequencer must include within N blocks or face penalties
   - Fallback mechanism if sequencer fails to include

2. **Deploy Security Council as multisig**
   - 8+ diverse signers
   - 75%+ threshold (e.g., 6/8)
   - Document emergency procedures

3. **Add upgrade timelock**
   - Minimum 7-day delay for non-emergency upgrades
   - Emergency path requires SC threshold

### For Future Stage 2 Transition

1. **Open proof submission**
   - Remove sequencer-only restriction on `verifyBatch()`
   - Anyone can submit valid proofs

2. **Extend exit window to 30+ days**
   - Mandatory delay on all upgrade paths
   - Clear user notification mechanism

3. **Formalize SC restrictions**
   - On-chain governance limiting SC powers
   - Transparent criteria for emergency actions

4. **Proof system maturity**
   - Comprehensive security audits
   - Multiple independent prover implementations
   - Operational track record

## Conclusion

ethrex L2 currently satisfies all **Stage 0** requirements and provides a solid foundation for rollup deployments.

The path to **Stage 1** is clear but requires implementing censorship-resistant withdrawals through a forced-inclusion mechanism. This is the primary gap preventing Stage 1 compliance.

**Stage 2** requires additional work on permissionless proving, extended exit windows, and formal restrictions on Security Council powers.

| Stage | Status | Primary Blocker |
|-------|--------|-----------------|
| Stage 0 | ✅ Met | - |
| Stage 1 | ❌ Not met | Forced inclusion for withdrawals |
| Stage 2 | ❌ Not met | Permissionless proving, 30-day exit window |

## References

### L2Beat Resources
- [L2Beat Stages Framework](https://l2beat.com/stages)
- [L2Beat Forum: Stages Update](https://forum.l2beat.com/t/stages-update-a-high-level-guiding-principle-for-stage-1/338)
- [L2Beat: Introducing Stages](https://medium.com/l2beat/introducing-stages-a-framework-to-evaluate-rollups-maturity-d290bb22befe)

### Rollup Comparisons
- [Taiko Alethia on L2Beat](https://l2beat.com/scaling/projects/taiko)
- [Scroll on L2Beat](https://l2beat.com/scaling/projects/scroll)
- [zkSync Era on L2Beat](https://l2beat.com/scaling/projects/zksync-era)
- [Starknet on L2Beat](https://l2beat.com/scaling/projects/starknet)
- [Arbitrum One on L2Beat](https://l2beat.com/scaling/projects/arbitrum)
- [Optimism on L2Beat](https://l2beat.com/scaling/projects/optimism)
- [Surge: A Based Rollup Template](https://www.nethermind.io/blog/surge-a-based-rollup-template-designed-for-ethereums-future)

### ethrex Documentation
- [ethrex Data Availability](./fundamentals/data_availability.md)
- [ethrex Withdrawals](./fundamentals/withdrawals.md)
- [ethrex Based Sequencing](./fundamentals/based.md)

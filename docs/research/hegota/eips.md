# Hegotá proposal catalog

Snapshot: Forkcast feed generated 2026-09-08T16:44:20.099Z. All 65 tracked numbered proposals have a profile below; 58 pass the captured ranking filter. No ethrex tiers have been assigned.

The EIP status describes document maturity; the Hegotá status describes fork inclusion. Layer labels below are Forkcast metadata, not a complete implementation-impact classification. Mechanisms summarize the pinned draft. Ethrex implications and review questions are analysis; open questions are not established vulnerabilities. Formal prerequisites are copied from each pinned EIP header.

[Overview](README.md) · [Dependencies and interactions](dependencies.md) · [Ethrex code map](ethrex.md) · [Source versions](sources.json)

| EIP | Proposal | Layer | Hegotá status | Ranking |
| --- | --- | --- | --- | --- |
| [2488](#eip-2488) | Deprecate the CALLCODE opcode | EL | Proposed | Rankable |
| [3298](#eip-3298) | Remove storage-clear refund and refund cap | EL | Proposed | Rankable |
| [4758](#eip-4758) | Deactivate SELFDESTRUCT | EL | Proposed | Rankable |
| [5920](#eip-5920) | PAY opcode | EL | Proposed | Rankable |
| [7645](#eip-7645) | Alias ORIGIN to SENDER | EL | Proposed | Rankable |
| [7666](#eip-7666) | EVM-ify the identity precompile | EL | Proposed | Rankable |
| [7668](#eip-7668) | Remove bloom filters | EL | Proposed | Rankable |
| [7709](#eip-7709) | Read BLOCKHASH from Storage and Update Cost | EL | Proposed | Rankable |
| [7716](#eip-7716) | Anti-correlation attestation penalties | CL | Proposed | Rankable |
| [7805](#eip-7805) | Fork-choice enforced Inclusion Lists (FOCIL) | CL | Scheduled | Scheduled headliner |
| [7807](#eip-7807) | SSZ execution blocks | EL | Proposed | Rankable |
| [7819](#eip-7819) | SETDELEGATE instruction | EL | Proposed | Rankable |
| [7851](#eip-7851) | Code-Controlled EOA Delegation | EL | Proposed | Rankable |
| [7862](#eip-7862) | Delayed State Root | EL | Proposed | Rankable |
| [7906](#eip-7906) | Transaction Assertions via State Diff Opcode | EL | Proposed | Rankable |
| [7923](#eip-7923) | Linear, Page-Based Memory Costing | EL | Proposed | Rankable |
| [7979](#eip-7979) | Call and Return Opcodes for the EVM | EL | Proposed | Rankable |
| [8015](#eip-8015) | Remove `deposit` and `eth1data` fields | CL | Proposed | Rankable |
| [8025](#eip-8025) | Optional Execution Proofs | CL | Proposed | Rankable |
| [8077](#eip-8077) | eth/XX - announce transactions with nonce | EL | Proposed | Rankable |
| [8094](#eip-8094) | eth/vhash - Blob-Aware Mempool | EL | Proposed | Rankable |
| [8105](#eip-8105) | Universal Enshrined Encrypted Mempool | EL | Withdrawn | Withdrawn |
| [8115](#eip-8115) | Batch priority fees at end of block | EL | Proposed | Rankable |
| [8116](#eip-8116) | Replace cumulative receipt fields | EL | Proposed | Rankable |
| [8131](#eip-8131) | Unified Transaction Content Floor | EL | Proposed | Rankable |
| [8141](#eip-8141) | Frame Transaction | EL | Scheduled | Scheduled |
| [8142](#eip-8142) | Block-in-Blobs (BiB) | CL | Proposed | Rankable |
| [8146](#eip-8146) | Block Access List Sidecars | CL | Proposed | Rankable |
| [8148](#eip-8148) | Custom sweep threshold for validators | CL | Proposed | Rankable |
| [8151](#eip-8151) | Account Code Restricted ecRecover | EL | Proposed | Rankable |
| [8163](#eip-8163) | Reserve `EXTENSION (0xae)` opcode | EL | Proposed | Rankable |
| [8173](#eip-8173) | Foundations of EVM Control Flow | EL | Proposed | Informational |
| [8182](#eip-8182) | Private ETH and ERC-20 Transfers | EL | Proposed | Rankable |
| [8184](#eip-8184) | LUCID encrypted mempool | EL | Declined | Declined |
| [8188](#eip-8188) | Last-Written Block for Accounts and Slots | EL | Proposed | Rankable |
| [8198](#eip-8198) | Quick Slots | CL | Proposed | Rankable |
| [8200](#eip-8200) | EVMification | EL | Proposed | Rankable |
| [8205](#eip-8205) | Withdrawal credentials preregistration | CL | Proposed | Rankable |
| [8219](#eip-8219) | Checked Arithmetic Opcodes | EL | Proposed | Rankable |
| [8237](#eip-8237) | Independent CL/EL Sync | CL | Proposed | Rankable |
| [8243](#eip-8243) | Batching Attestations at Source | CL | Proposed | Rankable |
| [8250](#eip-8250) | Keyed Nonces for Frame Transactions | EL | Proposed | Rankable |
| [8253](#eip-8253) | Bump nonce of zero-nonce storage accounts | EL | Proposed | Rankable |
| [8272](#eip-8272) | Recent Roots for Frame Transactions | EL | Proposed | Rankable |
| [8279](#eip-8279) | Block Access List Byte Floor | EL | Proposed | Rankable |
| [8298](#eip-8298) | SETCODEFROM Code Reuse Instruction | EL | Proposed | Rankable |
| [8304](#eip-8304) | Trustless log and transaction index | EL | Proposed | Rankable |
| [8321](#eip-8321) | Hash-Chain RANDAO | CL | Proposed | Rankable |
| [8333](#eip-8333) | Align Checkpoint with Epoch Boundary Block | CL | Proposed | Rankable |
| [8334](#eip-8334) | Bundled Attestation Propagation | CL | Proposed | Rankable |
| [8341](#eip-8341) | Partial Execution Payload Commitments | CL | Proposed | Rankable |
| [8355](#eip-8355) | Precompiles for ML-DSA Verification | EL | Proposed | Rankable |
| [8358](#eip-8358) | Net Gas Metering for Account Changes | EL | Proposed | Rankable |
| [8359](#eip-8359) | Beacon Block Reporting Field | CL | Proposed | Rankable |
| [8363](#eip-8363) | Tapered Issuance Burn | CL | Proposed | Rankable |
| [8365](#eip-8365) | BLS withdrawal credential retirement | CL | Proposed | Rankable |
| [8367](#eip-8367) | Balance sunset for retired BLS validators | CL | Proposed | Rankable |
| [8368](#eip-8368) | CPSB Recalibration for New Gas Limit | EL | Proposed | Rankable |
| [8369](#eip-8369) | VOPS Profiles for FOCIL Eligibility | CL | Proposed | Informational |
| [8371](#eip-8371) | RowDAS - Distributed Blob Reconstruction | CL | Proposed | Rankable |
| [8372](#eip-8372) | Normalized state gas limit | EL | Proposed | Rankable |
| [8374](#eip-8374) | Persist Warm Access Sets Across Reverts | EL | Proposed | Rankable |
| [8375](#eip-8375) | ePBS Mandatory Burn of Execution Rewards | CL | Proposed | Rankable |
| [8379](#eip-8379) | Top-up Sync | CL | Proposed | Rankable |
| [8383](#eip-8383) | Reduce CL Block Retention Window | CL | Proposed | Informational |

## EVM cleanup and control flow

### EIP-2488

**Deprecate the CALLCODE opcode** — Stagnant; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-2488.md) · [Discussion](https://ethereum-magicians.org/t/eip-2488-deprecate-the-callcode-opcode/3957) · [Forkcast](https://forkcast.org/eips/2488)

**Declared prerequisites:** [7](https://eips.ethereum.org/EIPS/eip-7).

**Mechanism.** Makes CALLCODE return failure (0), allowing the caller to handle failure; it does not turn the instruction into INVALID. DELEGATECALL already supplies the usual proxy semantics.

**Relationships.** Requires EIP-7, an existing prerequisite. Related to 7645 and 4758 as cleanup, but neither is required. 8173 provides control-flow background, not an implementation prerequisite.

**Ethrex implications.** Change CALLCODE dispatch/semantics in LEVM while retaining historical fork behavior. The patch is small; deployed-code compatibility is the expensive uncertainty.

**Review questions and draft gaps.** The stagnant draft still has unvalidated non-use claims and TBA security/tests. Obtain a current chain usage and failure-handling study before equating small implementation size with low risk.

### EIP-4758

**Deactivate SELFDESTRUCT** — Stagnant; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-4758.md) · [Discussion](https://ethereum-magicians.org/t/eip-4758-deactivate-selfdestruct/8710) · [Forkcast](https://forkcast.org/eips/4758)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Replaces SELFDESTRUCT with balance-transfer-only SENDALL: no code/storage deletion or nonce alteration. Against the post-6780 baseline, the main removed behavior is deletion of a contract created in the same transaction.

**Relationships.** No declared requires. 6780 is essential historical context, not a listed dependency. Simplifies account lifetime interactions with Frames, assertions, state accounting, and future trie changes. PAY (5920) is complementary, with an explicit amount and different control flow.

**Ethrex implications.** LEVM selfdestruct execution, deferred deletion, same-transaction creation tracking, BALs, transfer logs and frame rollback.

**Review questions and draft gaps.** The older prose says funds go to the caller, while the specification says target. Use the operand beneficiary interpretation only after confirming the composed execution specification. Test create-and-destroy patterns and self-target transfers.

### EIP-5920

**PAY opcode** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-5920.md) · [Discussion](https://ethereum-magicians.org/t/eip-5920-pay-opcode/11717) · [Forkcast](https://forkcast.org/eips/5920)

**Declared prerequisites:** [214](https://eips.ethereum.org/EIPS/eip-214), [2929](https://eips.ethereum.org/EIPS/eip-2929), [7523](https://eips.ethereum.org/EIPS/eip-7523).

**Mechanism.** Adds PAY (0xfc): transfers an explicit ETH amount without executing recipient code, returns success/failure, and rejects static context and addresses with nonzero high 96 bits. Insufficient balance returns 0.

**Relationships.** Requires 214, 2929, 7523. Complements 4758 and code-bearing accounts under 7702/8141. Composition with 7708 transfer logs and 8037/8038 or 8358 gas accounting needs attention.

**Ethrex implications.** A new LEVM handler using shared balance/account creation logic; include BAL writes, logs, rollback, and witness/prover execution.

**Review questions and draft gaps.** Non-calling transfers remove recipient reentrancy and rejection from this operation, but also bypass recipient hooks. Benchmark and settle costs against the target fork's schedule rather than copying older CALL constants.

### EIP-7645

**Alias ORIGIN to SENDER** — Stagnant; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7645.md) · [Discussion](https://ethereum-magicians.org/t/eip-7645-alias-origin-to-sender/19047) · [Forkcast](https://forkcast.org/eips/7645)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Globally makes ORIGIN return the current call's CALLER. In a multi-hop call it changes at every depth, affecting deployed contracts.

**Relationships.** Not required by Frames. EIP-8141 defines ORIGIN per top-level frame and keeps it constant through nested calls; that is a different rule. 7645 and 8141 need a precedence decision if combined.

**Ethrex implications.** A small environment-opcode edit with a broad application compatibility surface, including all call variants and authorization checks.

**Review questions and draft gaps.** A tx.origin == msg.sender gate becomes true at every depth, potentially removing a deployed guard. The draft simultaneously acknowledges breakage and says no compatibility issues were found; that is not supporting evidence of safety.

### EIP-7979

**Call and Return Opcodes for the EVM** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7979.md) · [Discussion](https://ethereum-magicians.org/t/eip-7979-call-and-return-opcodes-for-the-evm/24615) · [Forkcast](https://forkcast.org/eips/7979)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Adds CALLSUB, CALLDEST, and RETURNSUB with a separate return stack capped at 1024 entries. Proposed costs are 8, 1, and 5 gas. CALLSUB takes a destination from the operand stack and validates CALLDEST; CALLDEST also remains a valid ordinary jump target.

**Relationships.** No declared prerequisites. EIP-8173 explains the control-flow motivation. This does not by itself eliminate dynamic jumps or give arbitrary legacy code a fully static control-flow graph.

**Ethrex implications.** Opcode allocation/dispatch, jump-destination analysis, per-call return-stack state, tracing, compiler targets, and proof execution. Existing contracts benefit only if recompiled/deployed to use the instructions.

**Review questions and draft gaps.** Opcode values remain TBD. Verify return-stack underflow/overflow, destination scanning through PUSH data, and interactions with exceptional exits and Frame boundaries.

### EIP-8163

**Reserve `EXTENSION (0xae)` opcode** — Review; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8163.md) · [Discussion](https://ethereum-magicians.org/t/eip-8163-reserve-0xae-extension-opcode/27756) · [Forkcast](https://forkcast.org/eips/8163)

**Declared prerequisites:** [141](https://eips.ethereum.org/EIPS/eip-141).

**Mechanism.** Reserves byte 0xae as permanently INVALID on Ethereum L1, while permitting non-L1 environments to use it as an extension prefix without changing legacy jump-destination analysis.

**Relationships.** Requires 141. This is an opcode-namespace commitment, not a new L1 instruction or a prerequisite for EOF.

**Ethrex implications.** If 0xae is already invalid, no new L1 execution behavior is needed. Preserve the reservation in opcode allocation; assess separately if ethrex L2 experiments want an extension namespace.

**Review questions and draft gaps.** Do not count a policy reservation as a substantial new EVM feature. Non-L1 extension encodings must preserve the specified legacy byte-scanning behavior.

### EIP-8173

**Foundations of EVM Control Flow** — Draft; Informational; Hegotá: Proposed; Informational.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8173.md) · [Discussion](https://ethereum-magicians.org/t/eip-8173-foundations-of-evm-control-flow/27855) · [Forkcast](https://forkcast.org/eips/8173)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Informational analysis of EVM control flow, compiler constraints, static analysis, and proving. It discusses the motivation for improving calls and returns rather than specifying a fork transition.

**Relationships.** No declared prerequisites. Useful background for 7979 and broader EVM design choices; it does not make a control-flow instruction mandatory.

**Ethrex implications.** Provides criteria for reviewing interpreter/compiler/prover proposals. No direct consensus implementation task follows from this document.

**Review questions and draft gaps.** Excluded by Forkcast’s rankable filter because it is Informational. Keep explanatory arguments separate from the concrete semantics and maturity of standards-track proposals.

### EIP-8200

**EVMification** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8200.md) · [Discussion](https://ethereum-magicians.org/t/eip-8200-evmification/28036) · [Forkcast](https://forkcast.org/eips/8200)

**Declared prerequisites:** [152](https://eips.ethereum.org/EIPS/eip-152), [7666](eips.md#eip-7666), [7823](https://eips.ethereum.org/EIPS/eip-7823), [7883](https://eips.ethereum.org/EIPS/eip-7883).

**Mechanism.** Replaces RIPEMD-160, MODEXP, and BLAKE2f native precompiles with ordinary EVM bytecode at their addresses. Like identity replacement, this moves semantics and resource charging into normal execution.

**Relationships.** Requires 152, 7666, 7823, and 7883. 7666 is the direct in-roster prerequisite. 8355 adds a native cryptographic precompile: a different design tradeoff, not a formal incompatibility.

**Ethrex implications.** Precompile dispatch, fork-time code installation, native-vs-bytecode failure behavior, gas estimation, and prover workload. Historical forks still need the old implementations.

**Review questions and draft gaps.** Replacement bytecode is not specified yet. Require audited code and worst-case gas/runtime/proof benchmarks; ordinary EVM execution can break contracts that supplied gas based on the old precompile schedule.

### EIP-8219

**Checked Arithmetic Opcodes** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8219.md) · [Discussion](https://ethereum-magicians.org/t/eip-8219-checked-arithmetic-opcodes/27913) · [Forkcast](https://forkcast.org/eips/8219)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Adds unsigned checked SAFEADD, SAFESUB, SAFEMUL, and SAFEDIV at 0x0c–0x0f, costing 5/5/7/7 gas. Overflow, underflow, or division by zero reverts with empty data, instead of wrapping or returning zero.

**Relationships.** No declared prerequisites. A compiler-target feature, independent of subroutines 7979 or Frame transactions.

**Ethrex implications.** Four opcode handlers, fork tables, stack/error semantics, tracing, and proving. Benefits require newly compiled code; existing arithmetic keeps its behavior.

**Review questions and draft gaps.** Use REVERT semantics rather than exceptional gas-consuming failure. Compiler-generated panic payloads differ from empty revert data, so adoption must be intentional.

### EIP-8355

**Precompiles for ML-DSA Verification** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8355.md) · [Discussion](https://ethereum-magicians.org/t/eip-8355-precompiles-for-ml-dsa-verification/29211) · [Forkcast](https://forkcast.org/eips/8355)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Adds ML-DSA-44/65/87 verification precompiles at 0x12/0x13/0x14, with costs 6500/9000/13500 plus six gas per message word. Input contains fixed-size public key and signature followed by the message; empty context is used. Invalid input returns a zero word, valid signatures return one.

**Relationships.** No declared prerequisites. Can support 8141 account authentication without being required by Frames. Adds native cryptography while 8200 removes other native precompiles; assess the tradeoff individually.

**Ethrex implications.** Cryptographic library integration, exact FIPS-204 encoding/validation, precompile gas and dispatch, vectors, and every prover backend. Protocol support does not migrate existing keys automatically.

**Review questions and draft gaps.** Benchmark adversarial invalid inputs and circuit/prover costs as well as native throughput. Audit canonical decoding, address allocation, and the precise variant/message conventions against the normative cryptographic standard.


## Gas accounting and resource bounds

### EIP-3298

**Remove storage-clear refund and refund cap** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-3298.md) · [Discussion](https://ethereum-magicians.org/t/eip-3298-removal-of-refunds/5430) · [Forkcast](https://forkcast.org/eips/3298)

**Declared prerequisites:** [2780](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-2780.md), [3529](https://eips.ethereum.org/EIPS/eip-3529), [7778](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7778.md), [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md), [8038](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8038.md).

**Mechanism.** Current draft removes the storage-clearing incentive refund and the 20% transaction refund cap. It preserves the net-metered STORAGE_WRITE refund for restoring a slot to its transaction-start value. State-gas refills remain separate.

**Relationships.** Explicit delta over 8037/8038, assuming 2780 and 7778. Must compose with 8131/8279 floors, 8141 settlement, and proposed 8358 account refunds. It does not delete all refunds.

**Ethrex implications.** LEVM SSTORE accounting, transaction settlement, receipt gas, and RPC estimates. Existing frame settlement must also adopt the intended composition; current 8141 still specifies a refund cap.

**Review questions and draft gaps.** Verify every remaining refund is backed by a same-transaction charge and reverts correctly. Check legacy and Frames payment versus block-capacity accounting separately. 7819 still proposes an existing-account refund: the combined schedule needs explicit reconciliation.

### EIP-7709

**Read BLOCKHASH from Storage and Update Cost** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7709.md) · [Discussion](https://ethereum-magicians.org/t/eip-7709-read-blockhash-opcode-from-storage-and-adjust-gas-cost/20052) · [Forkcast](https://forkcast.org/eips/7709)

**Declared prerequisites:** [2935](https://eips.ethereum.org/EIPS/eip-2935).

**Mechanism.** Serves in-window BLOCKHASH from 2935 history-contract storage with SLOAD gas, warming, and state-access effects, in addition to BLOCKHASH's base charge. The opcode still exposes only 256 ancestors; the storage ring has 8191 entries.

**Relationships.** Requires 2935 sufficiently before activation, or at genesis. Interacts with 7928 BAL recording, stateless witnesses, and 8374 access-set rollback policy.

**Ethrex implications.** BLOCKHASH handler, database/history lookup, witness construction, and BAL recording. A cached lookup remains permitted only if all storage-access semantics are reproduced.

**Review questions and draft gaps.** Cover out-of-window arguments without added storage effects, repeated and reverted reads, and networks with insufficient preactivation history. Existing contracts with tightly budgeted BLOCKHASH calls can fail after repricing.

### EIP-7923

**Linear, Page-Based Memory Costing** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7923.md) · [Discussion](https://ethereum-magicians.org/t/eip-linearize-memory-costing/23290) · [Forkcast](https://forkcast.org/eips/7923)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Replaces quadratic memory expansion with 4 KiB pages at 100 gas each, with the first page free; introduces a 32-bit address space and a 64 MiB transaction-wide allocation bound. This changes memory semantics and resource limits, not just a gas constant.

**Relationships.** No declared prerequisites. Shares the resource-bounding goal of 8131/8279, but measures a different resource. All new and existing opcodes, nested calls, precompiles, and Frame transactions need consistent memory accounting.

**Ethrex implications.** LEVM memory allocation, call-frame lifetime, gas calculation, exceptional halts, and prover memory/witness behavior. Establish worst-case resident memory and execution time before accepting the proposed price.

**Review questions and draft gaps.** The reference implementation releases child pages on return; the precise global lifetime and boundary rules need review. Old block-gas assumptions do not establish safety at the target fork limits.

### EIP-8131

**Unified Transaction Content Floor** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8131.md) · [Discussion](https://ethereum-magicians.org/t/eip-9999-add-auth-data-to-eip-7623-floor/12345) · [Forkcast](https://forkcast.org/eips/8131)

**Declared prerequisites:** [2028](https://eips.ethereum.org/EIPS/eip-2028), [4844](https://eips.ethereum.org/EIPS/eip-4844), [7623](https://eips.ethereum.org/EIPS/eip-7623), [7702](https://eips.ethereum.org/EIPS/eip-7702), [7976](https://eips.ethereum.org/EIPS/eip-7976), [7981](https://eips.ethereum.org/EIPS/eip-7981).

**Mechanism.** Applies a uniform 64-gas-per-byte floor to calldata, access lists, authorizations, and blob hashes, independent of byte values. This is a transaction-content floor, not a universal increase of intrinsic gas for every byte.

**Relationships.** Requires existing calldata/blob/delegation floor and size rules. 8279 explicitly extends this accounting to BAL bytes; 8141 needs a current definition of all counted Frame and signature fields.

**Ethrex implications.** Transaction content-size calculation, floor settlement, gas estimation, pool checks, block construction, and proof validation. Claimed byte bounds cover the defined user content, not every network or block byte.

**Review questions and draft gaps.** The draft uses older intrinsic/authorization conventions. Reconcile 2780/8037 accounting and current transaction schemas before applying its formulas literally.

### EIP-8279

**Block Access List Byte Floor** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8279.md) · [Discussion](https://ethereum-magicians.org/t/eip-8279-block-access-list-byte-floor/28662) · [Forkcast](https://forkcast.org/eips/8279)

**Declared prerequisites:** [7623](https://eips.ethereum.org/EIPS/eip-7623), [7702](https://eips.ethereum.org/EIPS/eip-7702), [7928](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7928.md), [7976](https://eips.ethereum.org/EIPS/eip-7976), [7981](https://eips.ethereum.org/EIPS/eip-7981), [8131](eips.md#eip-8131).

**Mechanism.** Extends the 8131 content floor to measured BAL bytes. Counters are conservative across reverts; exceeding the floor budget can cause runtime out-of-gas even when ordinary execution gas remains. Storage-value accounting can adjust for net writes separately from ordinary gas refunds.

**Relationships.** Requires 7623, 7702, 7928, 7976, 7981, and 8131. Must compose with 8037 dual gas, 3298/8358 refunds, 8374 warming, and current Frame semantics. System BAL work retains a separate block-level bound.

**Ethrex implications.** Instrument execution and BAL builders together, enforce runtime failure at the correct operation, and align pool estimation, receipts, block limits, and proof witnesses.

**Review questions and draft gaps.** Reverted execution can still consume the conservative byte budget. Validate all byte-count formulas and authorizations against the final fork constants and 8141 schema; do not assume the floor can be calculated only after successful execution.

### EIP-8358

**Net Gas Metering for Account Changes** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/rakita/EIPs/blob/c656c4e42c3c66db843f82035d521abc8e836b51/EIPS/eip-8358.md) · [Open EIPs PR #12058](https://github.com/ethereum/EIPs/pull/12058) · [Discussion](https://ethereum-magicians.org/t/eip-8358-net-gas-metering-for-account-changes/29304) · [Forkcast](https://forkcast.org/eips/8358)

**Declared prerequisites:** [161](https://eips.ethereum.org/EIPS/eip-161), [2200](https://eips.ethereum.org/EIPS/eip-2200), [2780](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-2780.md), [2929](https://eips.ethereum.org/EIPS/eip-2929), [3529](https://eips.ethereum.org/EIPS/eip-3529), [7702](https://eips.ethereum.org/EIPS/eip-7702), [7708](https://eips.ethereum.org/EIPS/eip-7708), [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md), [8038](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8038.md).

**Mechanism.** Applies net metering to account balance/nonce writes: lower cost for already-dirty accounts and refunds when the original values are restored. Also changes value-call stipend calculation from an additive stipend to a minimum forwarded-gas floor.

**Relationships.** Requires 161, 2200, 2780, 2929, 3529, 7702, 7708, 8037, and 8038. Compose with 3298 refund-cap removal, 8298 account writes, 8115/8375 fee processing, and 8141 transaction/frame baselines. 8374 is related but not a declared prerequisite.

**Ethrex implications.** Original-state tracking, call/create/authorization paths, balance and nonce journals, refunds, transaction settlement, and proof witnesses.

**Review questions and draft gaps.** The stipend change can forward 2300 less gas than before when the requested amount already exceeds 2300. Review fixed-gas callers despite broad compatibility claims. Define when original account values are captured relative to intrinsic effects, authorizations, and Frame approvals.

### EIP-8368

**CPSB Recalibration for New Gas Limit** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8368.md) · [Discussion](https://ethereum-magicians.org/t/eip-8368-cpsb-recalibration-for-new-gas-limit/29293) · [Forkcast](https://forkcast.org/eips/8368)

**Declared prerequisites:** [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md).

**Mechanism.** Proposes recalibrating the state-creation cost per byte for a new reference block gas limit. The reference limit and resulting values are TBD.

**Relationships.** Requires 8037. Related to 8372, which separates raw state capacity using a scaling factor. They are parameter-design alternatives or possible coordinated changes, not declared prerequisites of each other.

**Ethrex implications.** Gas schedules, configuration, state-growth benchmarks, payload capacity, estimation, and proof validation. Ethrex currently returns a fixed cost-per-state-byte value of 1530.

**Review questions and draft gaps.** There is no final numeric change to assess yet. Require explicit growth targets and benchmarks, then evaluate together with Frame state limits and the chosen gas-limit trajectory.

### EIP-8372

**Normalized state gas limit** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8372.md) · [Discussion](https://ethereum-magicians.org/t/eip-8372-normalized-state-gas-limit/29332) · [Forkcast](https://forkcast.org/eips/8372)

**Declared prerequisites:** [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md).

**Mechanism.** Introduces a scale factor for raw state gas, with gas_used = max(execution_gas, floor(raw_state_gas * 100 / scale_factor)). Recalibrates the state-byte cost and limit while keeping existing header and legacy receipt shapes.

**Relationships.** Requires 8037. Related to 8368’s recalibration and current 8141 two-dimensional limits. This is not a separate fee market such as 7999.

**Ethrex implications.** Two-pool gas accounting, block capacity/reservations, transaction settlement, builder packing, RPC estimates, and proof execution. No new header field does not mean the accounting change is local.

**Review questions and draft gaps.** Both scale and byte price are TBD. Check integer-rounding boundaries and residual block capacity, particularly with mixed legacy and Frame transactions and floor/refund proposals.

### EIP-8374

**Persist Warm Access Sets Across Reverts** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/rakita/EIPs/blob/6c39b7f74eaf2fe2b7f678a0da8441822bcad007/EIPS/eip-8374.md) · [Open EIPs PR #12128](https://github.com/ethereum/EIPs/pull/12128) · [Discussion](https://ethereum-magicians.org/t/eip-8374-persist-warm-access-sets-across-reverts/29341) · [Forkcast](https://forkcast.org/eips/8374)

**Declared prerequisites:** [2929](https://eips.ethereum.org/EIPS/eip-2929).

**Mechanism.** Makes accessed-address and accessed-storage-key warm sets append-only through reverts, so an access warmed in a failed child call remains warm afterward. Ordinary state changes still revert.

**Relationships.** Requires 2929. Directly conflicts with 8141’s current explicit reverted-warming behavior unless composed as an amendment. Also affects new reads in 7709/8151 and interacts with 8279’s separate conservative BAL accounting.

**Ethrex implications.** LEVM access-set journals and rollback, tracing, gas estimation, and witness consistency. Keep state rollback separate from access-set rollback.

**Review questions and draft gaps.** Lower local access charges can change gas-dependent control flow, so avoid claiming every full transaction is behaviorally unchanged or cheaper. Verify nested reverts, batches, failed VERIFY frames, and historical fork behavior.


## Precompiles and cryptography

### EIP-7666

**EVM-ify the identity precompile** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7666.md) · [Discussion](https://ethereum-magicians.org/t/eip-7561-evm-ify-the-identity-precompile/19445) · [Forkcast](https://forkcast.org/eips/7666)

**Declared prerequisites:** [3855](https://eips.ethereum.org/EIPS/eip-3855).

**Mechanism.** Installs seven bytes of copying/returning EVM code at address 0x04 and stops treating it as the identity precompile. The returned data is equivalent; gas and account-code observability change.

**Relationships.** Requires PUSH0 (3855). Explicit prerequisite of 8200. MCOPY is motivation, not a dependency of the supplied bytecode.

**Ethrex implications.** Fork activation code installation, precompile membership, call costs, EXTCODE observations, BAL/witness access, and prover behavior.

**Review questions and draft gaps.** Compare empty and large inputs, low gas, warm/cold behavior, and introspection. Ordinary EVM memory expansion can change cost materially for large inputs; output equivalence alone does not establish compatibility.


## Blocks, receipts, and state

### EIP-7668

**Remove bloom filters** — Stagnant; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7668.md) · [Discussion](https://ethereum-magicians.org/t/eip-7653-remove-bloom-filters/19447) · [Forkcast](https://forkcast.org/eips/7668)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Requires block and receipt bloom fields to be zero bytes long. It retains the fields rather than deleting them, and does not lower LOG gas.

**Relationships.** Related to 8116 receipt changes and 8304 alternate indexing. Neither is a hard dependency. 7807 also removes the block bloom but specifies all-one JSON-RPC bloom output and leaves receipt blooms alone, so combined rules need reconciliation.

**Ethrex implications.** Fixed Bloom types, header and receipt encoding/hashing, wire formats, RPC output, and eth_getLogs filtering.

**Review questions and draft gaps.** Preserve historical bloom queries and provide a usable local index path. Empty serialized bytes, a 256-byte all-zero bloom, and an all-one fallback are different representations with different effects on log consumers.

### EIP-7807

**SSZ execution blocks** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7807.md) · [Discussion](https://ethereum-magicians.org/t/eip-7807-ssz-execution-blocks/21580) · [Forkcast](https://forkcast.org/eips/7807)

**Declared prerequisites:** [7495](https://eips.ethereum.org/EIPS/eip-7495), [7773](https://eips.ethereum.org/EIPS/eip-7773), [7916](https://eips.ethereum.org/EIPS/eip-7916).

**Mechanism.** Migrates execution block/header hashing and per-block commitments to progressive SSZ structures. Individual transaction/receipt envelopes and the account state trie retain their existing encodings.

**Relationships.** Requires 7495, 7773, 7916. Overlaps 7668, 8116, 8304, and payload-commitment changes without automatically depending on them.

**Ethrex implications.** Header/block types, hashes, transaction and receipt commitments, networking, Engine API, RPC, historical decoding and proof tools; broad integration work.

**Review questions and draft gaps.** Block hashes become SSZ SHA-256 roots, not keccak(RLP). Generic header-proof consumers break. The draft requires all-one RPC block blooms, not empty receipt blooms; do not conflate it with 7668.

### EIP-7862

**Delayed State Root** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7862.md) · [Discussion](https://ethereum-magicians.org/t/eip-7862-delayed-execution-layer-state-root/22559) · [Forkcast](https://forkcast.org/eips/7862)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Changes block n's state_root to commit to the post-state of n-1. This removes the current block's state-root computation from its production/validation critical path, with a one-block delay for authenticated post-state proofs.

**Relationships.** No declared requires. ePBS and BALs motivate the change. 8341 tackles repeated builder-root computation by changing bid commitments while retaining final payload-root semantics: an overlapping approach, not a dependency.

**Ethrex implications.** Header validation, trie scheduling, payload building, reorg caches, snapshot/snap sync, RPC state proofs and stateless/prover input-output commitments.

**Review questions and draft gaps.** Activation and branch-specific state-root tracking must be exact. Audit all consumers that assume a block header authenticates that same block's resulting state.

### EIP-8115

**Batch priority fees at end of block** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8115.md) · [Discussion](https://ethereum-magicians.org/t/eip-8115-batch-priority-fees-at-end-of-block/27358) · [Forkcast](https://forkcast.org/eips/8115)

**Declared prerequisites:** [1559](https://eips.ethereum.org/EIPS/eip-1559), [4895](https://eips.ethereum.org/EIPS/eip-4895).

**Mechanism.** Accumulates transaction priority fees during execution and credits the fee recipient once after all transactions and before withdrawals. This removes repeated fee-recipient balance writes.

**Relationships.** Requires 1559 and 4895. Shares the parallel-execution goal with 8116. Must explicitly compose with 7708 transfer logs, 8141 fee settlement, and 8375 per-transaction priority-fee burning.

**Ethrex implications.** Block execution/finalization, transaction hooks, BAL ordering, receipts/logs, and parallel execution scheduling. Contracts can no longer spend fees earned earlier in the same block before the final credit.

**Review questions and draft gaps.** Check existing coinbase-sensitive transactions and builders. Specify exact system-operation ordering and failure behavior; the compatibility impact is observable despite unchanged total fees.

### EIP-8116

**Replace cumulative receipt fields** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8116.md) · [Discussion](https://ethereum-magicians.org/t/eip-8116-replace-cumulative-receipt-fields/27359) · [Forkcast](https://forkcast.org/eips/8116)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Replaces cumulative gas used with per-transaction gas used in consensus receipts and changes RPC log indexing to be relative to the receipt. This removes dependencies on earlier receipts.

**Relationships.** No declared prerequisites. Shares motivations with 8115 and requires deliberate composition with 7807 receipt roots and 8141 nested frame receipts.

**Ethrex implications.** Receipt types/encoding, receipt roots, RPC responses, P2P receipt serving, explorers/indexers, and tools expecting block-global log indices.

**Review questions and draft gaps.** The draft example transactionIndex * 4 + logIndex does not preserve order for transactions with more than four logs. Consumers should use the actual lexicographic pair, and Frames require an explicit frame/log ordering rule.

### EIP-8188

**Last-Written Block for Accounts and Slots** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8188.md) · [Discussion](https://ethereum-magicians.org/t/eip8188-state-tiering-by-write-age/28234) · [Forkcast](https://forkcast.org/eips/8188)

**Declared prerequisites:** [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md).

**Mechanism.** Adds last_written_block metadata to account and storage encodings. Legacy records decode with timestamp zero and migrate lazily; actual writes stamp the block number, reverted changes restore prior metadata, and zero-valued storage is deleted.

**Relationships.** Requires 8037. Current proposal is write-history metadata, not SSTORE gas repricing or state expiry. It could support later history/expiry designs but does not implement them.

**Ethrex implications.** Account RLP, storage-value encoding, trie roots, database adapters, snap sync, state proofs, journals, witnesses, and proving. Changes that alter storage must update the owning account consistently.

**Review questions and draft gaps.** Define actual changes versus no-op writes across every mutation path, including code/delegation and system calls. Test mixed legacy/new encodings and state-root agreement at activation; assess ongoing metadata growth.

### EIP-8253

**Bump nonce of zero-nonce storage accounts** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8253.md) · [Discussion](https://ethereum-magicians.org/t/eip-8253-upgrade-pre-spurious-dragon-accounts/28505) · [Forkcast](https://forkcast.org/eips/8253)

**Declared prerequisites:** [161](https://eips.ethereum.org/EIPS/eip-161), [684](https://eips.ethereum.org/EIPS/eip-684).

**Mechanism.** At activation, sets the nonce to one for a fixed list of 28 mainnet accounts with empty code, zero nonce, and nonempty storage. Executes before other system operations without gas or logs; BAL changes use index zero.

**Relationships.** Requires 161 and 684. An alternative cleanup approach to changing creation-collision checks under 7610. May simplify later state designs but is not a mandatory prerequisite for every trie change.

**Ethrex implications.** Chain-specific fork migration, account updates, state-root calculation, BAL/system ordering, and stateless witnesses. Preserve all other account/storage fields.

**Review questions and draft gaps.** Rederive/verify the exceptional-account list against the actual activation state, and define non-mainnet behavior. The draft’s fixed list is not evidence that future chains contain exactly the same exceptions.

### EIP-8304

**Trustless log and transaction index** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8304.md) · [Discussion](https://ethereum-magicians.org/t/eip-8304-trustless-log-and-transaction-index/28824) · [Forkcast](https://forkcast.org/eips/8304)

**Declared prerequisites:** [4788](https://eips.ethereum.org/EIPS/eip-4788).

**Mechanism.** Builds authenticated sorted log and transaction indices in a system contract, using SSZ trees and ring-buffer levels covering 1, 4, 16, 64, and 256 blocks. Larger-level merges are delayed; parent-block hashing avoids a self-reference cycle.

**Relationships.** Requires 4788. Complements bloom removal 7668 without requiring it. Optional Frame indices must match 8141 receipt/log semantics, and 8116 indexing needs an explicit composition.

**Ethrex implications.** Post-block/system processing, historical receipt access, contract state, index generation, state proofs, sync bootstrap data, and proof execution. The draft requires recent historical material beyond just the current state.

**Review questions and draft gaps.** Contract address/bytecode remain unfinished. Verify completeness proofs, sorting/duplicate rules, delayed merge cost, history availability after sync, and silent-failure behavior if the system contract is absent.


## Consensus, validators, and economics

### EIP-7716

**Anti-correlation attestation penalties** — Stagnant; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7716.md) · [Discussion](https://ethereum-magicians.org/t/eip-7716-anti-correlation-attestation-penalties/20137) · [Forkcast](https://forkcast.org/eips/7716)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Scales missed-attestation penalties using aggregate nonparticipation and a running excess-penalty accumulator, with a maximum multiplier of four. It aims to penalize correlated outages more than isolated failures.

**Relationships.** No declared dependencies. Related to diversification and consensus economics; not a FOCIL or Frames prerequisite. This concerns ordinary attestation penalties, not a new slashable offense.

**Ethrex implications.** Primarily CL state/reward processing. For ethrex, assess chain liveness and testing coordination rather than assume direct LEVM implementation.

**Review questions and draft gaps.** The stagnant draft has sparse integration detail and acknowledges view-splitting attacks. Require calibrated outage simulations, treatment of network-wide failures, and complete epoch-transition rules.

### EIP-8015

**Remove `deposit` and `eth1data` fields** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8015.md) · [Discussion](https://ethereum-magicians.org/t/eip-8015-remove-legacy-deposit-and-eth1data-fields/25401) · [Forkcast](https://forkcast.org/eips/8015)

**Declared prerequisites:** [6110](https://eips.ethereum.org/EIPS/eip-6110), [7688](https://eips.ethereum.org/EIPS/eip-7688), [7773](https://eips.ethereum.org/EIPS/eip-7773).

**Mechanism.** Removes obsolete eth1_data/deposit fields from BeaconBlockBody and BeaconState after the legacy deposit mechanism is drained. New deposits continue through EIP-6110 execution requests.

**Relationships.** Requires 6110, 7688, and 7773. Progressive containers preserve unrelated generalized indices; the cleanup must follow the prior deposit transition.

**Ethrex implications.** Primarily a CL schema/state migration. Check consensus fixtures and any local beacon SSZ/proof consumers; it does not remove ethrex deposit request processing.

**Review questions and draft gaps.** Activation must respect legacy deposit completion and fork-aware SSZ decoding. Distinguish schema cleanup from changes to users’ ability to deposit.

### EIP-8148

**Custom sweep threshold for validators** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8148.md) · [Discussion](https://ethereum-magicians.org/t/eip-8148-custom-sweep-threshold-for-validators/27669) · [Forkcast](https://forkcast.org/eips/8148)

**Declared prerequisites:** [7251](https://eips.ethereum.org/EIPS/eip-7251), [7685](https://eips.ethereum.org/EIPS/eip-7685).

**Mechanism.** Allows compounding validators to choose a withdrawal sweep threshold and makes that threshold cap effective balance. Adds an EL request contract/queue, request type 0x03, and CL state/processing; requests carry source address, validator pubkey, and threshold.

**Relationships.** Requires 7251 and 7685. Operates on compounding 0x02 credentials, unlike 8365/8367’s legacy BLS credentials. It is not solely a CL change despite its subject matter.

**Ethrex implications.** Install and call the request contract, extract/encode requests, validate Engine data, and include system execution in BAL/witness handling. Proposed queue limits are target 2 and maximum 16 requests per block.

**Review questions and draft gaps.** Contract address/deployment details require finalization. Check threshold bounds, byte order between user input and request encoding, system-call gas, and interactions with pending deposits/withdrawals.

### EIP-8198

**Quick Slots** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8198.md) · [Discussion](https://ethereum-magicians.org/t/eip-8198-quick-slots/28057) · [Forkcast](https://forkcast.org/eips/8198)

**Declared prerequisites:** [7892](https://eips.ethereum.org/EIPS/eip-7892).

**Mechanism.** Current Quick Slots draft changes slot duration from 12 to 8 seconds. It scales block gas by two-thirds once at activation and adjusts blob limits and CL per-time parameters so shorter latency need not increase throughput.

**Relationships.** Requires 7892. Interacts with every slot/epoch deadline: FOCIL/ePBS, 8025 proving, 8371 reconstruction, 8363 issuance, and 8383 retention. Older descriptions mentioning 10 seconds are stale.

**Ethrex implications.** Fork schedules/timestamps, first-block gas-limit validation, BPO blob schedules, Engine coordination, and latency testing. EL code should consume configured timing rather than assume 12 seconds.

**Review questions and draft gaps.** Handle skipped activation slots and the one-time gas-voting exemption exactly. Recalculate wall-clock targets for churn, rewards, retention, and proof deadlines; faster slots are not free additional execution capacity.

### EIP-8205

**Withdrawal credentials preregistration** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8205.md) · [Discussion](https://ethereum-magicians.org/t/eip-8205-withdrawal-credentials-preregistration/28084) · [Forkcast](https://forkcast.org/eips/8205)

**Declared prerequisites:** [6110](https://eips.ethereum.org/EIPS/eip-6110), [7685](https://eips.ethereum.org/EIPS/eip-7685), [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md).

**Mechanism.** Adds expiring BLS-signed withdrawal-credential preregistration through an EL request queue and CL records. A new validator deposit with mismatched credentials is ignored rather than credited; top-ups are unaffected. Proposed record lifetime is 262144 slots.

**Relationships.** Requires 6110, 7685, and 7732. Addresses first-deposit credential substitution, distinct from custom sweep thresholds 8148 and retirement of existing BLS credentials 8365.

**Ethrex implications.** Request contract deployment/collection, Engine encoding, system execution, and fork fixtures. The new request type and contract details remain to be finalized.

**Review questions and draft gaps.** Deposits are processed before preregistrations in the same payload: protection must already exist and be checked atomically against a recent beacon state. Replayed signatures can refresh records; model registry growth. A rejected mismatched deposit does not imply a refund.

### EIP-8243

**Batching Attestations at Source** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8243.md) · [Discussion](https://ethereum-magicians.org/t/eip-8243-batching-attestations-at-source/28606) · [Forkcast](https://forkcast.org/eips/8243)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Lets validators sharing attestation data authorize a batcher to aggregate at the source. Batch seals and a batcher signature support a WireAttestation union while leaving on-chain attestations unchanged.

**Relationships.** No declared prerequisites. 8334 bundles separate unaggregated signatures using partial-message gossip; the two reduce network overhead at different points and need compatible wire/gossip rules.

**Ethrex implications.** Primarily CL validator and gossip software. For ethrex, relevant to integration/performance assumptions and any deployment that colocates validator infrastructure.

**Review questions and draft gaps.** Validate overlap/deduplication, batcher failure/failover, key custody, and operator-linkability risks. Authorization does not create a new slashing rule or prove independent operators.

### EIP-8321

**Hash-Chain RANDAO** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8321.md) · [Discussion](https://ethereum-magicians.org/t/eip-8321-hash-chain-randao/28942) · [Forkcast](https://forkcast.org/eips/8321)

**Declared prerequisites:** [7916](https://eips.ethereum.org/EIPS/eip-7916).

**Mechanism.** Replaces participating validators’ RANDAO reveals with a registered BLAKE3 hash chain, activated after three epochs. Registration is bounded to 128 per block; unregistered validators continue using legacy BLS reveals. Commitments cannot be reset without exit and reentry.

**Relationships.** Requires 7916. Changes the randomness contribution mechanism while preserving the EL PREVRANDAO interface. It is not a complete post-quantum validator-signature migration.

**Ethrex implications.** Primarily CL registration/state/gossip and validator key management. Assess downstream assumptions about randomness and fork fixtures; no new EVM opcode is implied.

**Review questions and draft gaps.** Review registration grinding, orphaned reveals exposing future values, chain exhaustion, and operational recovery. Its proposed domain 0x0F000000 also appears in 8375, which explicitly requires deconfliction before activation.

### EIP-8333

**Align Checkpoint with Epoch Boundary Block** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8333.md) · [Discussion](https://ethereum-magicians.org/t/eip-8333-align-checkpoint-with-epoch-boundary-block/29003) · [Forkcast](https://forkcast.org/eips/8333)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Moves an epoch’s FFG checkpoint root to the last block before that epoch starts, with fallback through empty slots. Epoch identifiers stay the same.

**Relationships.** No declared prerequisites. Related to sync/checkpoint consumers and other finality timing work; not an EL state-transition change.

**Ethrex implications.** CL fork-choice/state transition and checkpoint interfaces; ethrex integration tests may need updated finalized/head sequences.

**Review questions and draft gaps.** Cross-fork attestations must use the appropriate old/new checkpoint-root rule. Test empty boundary slots, multi-epoch gaps, justification/finalization, and checkpoint-provider interoperability.

### EIP-8334

**Bundled Attestation Propagation** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/sukunrt/EIPs/blob/09b25ff64900f729d845c120afa44c0f21b7ce5e/EIPS/eip-8334.md) · [Open EIPs PR #11905](https://github.com/ethereum/EIPs/pull/11905) · [Discussion](https://ethereum-magicians.org/t/eip-8334-bundled-attestation-propagation/29008) · [Forkcast](https://forkcast.org/eips/8334)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Uses peer-negotiated gossipsub partial messages to bundle up to 50 attestations with common data while retaining individual signatures. Receivers advertise/request missing validator indices; the draft recommends a 20 ms collection delay.

**Relationships.** No declared EIP prerequisites, but it relies on partial-message gossip support. 8243 uses source aggregation instead; combined operation requires an explicit wire and deduplication design.

**Ethrex implications.** CL networking rather than LEVM. Test the resulting arrival-time distribution against shared block/attestation deadlines and realistic mixed-capability peers.

**Review questions and draft gaps.** Deduplication must not let an invalid signature poison valid attestations with the same data/index. Benchmark CPU, bandwidth, and tail latency; peer negotiation makes this different from a mandatory state-transition change.

### EIP-8359

**Beacon Block Reporting Field** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/hanniabu/EIPs/blob/06ee26f2b86f36c839f9c41d99fcf49ab8323e5d/EIPS/eip-8359.md) · [Open EIPs PR #12063](https://github.com/ethereum/EIPs/pull/12063) · [Discussion](https://ethereum-magicians.org/t/eip-8359-beacon-block-reporting-field/29224) · [Forkcast](https://forkcast.org/eips/8359)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Adds a 32-byte client_setup_info field to beacon blocks for voluntary reporting of CL/EL client pairs and validator arrangements. Any 32-byte value is valid; the protocol does not verify its truth.

**Relationships.** No declared prerequisites. A consensus container change carrying informational data, distinct from an Informational EIP classification.

**Ethrex implications.** Primarily CL block schema/configuration. Ethrex appears in the proposed EL registry; operators may configure reports, but these reports are not authenticated measurements of actual client diversity.

**Review questions and draft gaps.** Weigh schema cost, operator privacy, and self-reporting bias. The security discussion questions reserved values, whereas the normative rule accepts any value; settle that discrepancy before implementation.

### EIP-8363

**Tapered Issuance Burn** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8363.md) · [Discussion](https://ethereum-magicians.org/t/eip-8363-tapered-issuance-burn/29263) · [Forkcast](https://forkcast.org/eips/8363)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Burns a fraction of assigned validator-duty rewards based on total active stake, with normative curve b = min(1, (D / D_sat)^1.5) and proposed fixed saturation 60.25 million ETH. Temporarily raises the base reward factor from 64 to 128 and tapers it over 123300 epochs. Attestation burning is suspended during inactivity leaks.

**Relationships.** No declared prerequisites. Interacts economically with 7716 penalties and technically with 8198 reward-factor and epoch-duration changes. 8375 burns execution-related proceeds through a separate mechanism.

**Ethrex implications.** CL reward/penalty accounting and fork fixtures; indirect validator incentives and network participation affect ethrex operation. No direct EVM change follows.

**Review questions and draft gaps.** The abstract’s linear description differs from the formula. Model net rewards, duty failures, saturation, and the combined parameter transition; do not equate an issuance claim with demonstrated behavioral or security outcomes.

### EIP-8365

**BLS withdrawal credential retirement** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ensi321/EIPs/blob/3f54e8705275e6ea4e1957c6a4dd99ec00a7bbe7/EIPS/eip-8365.md) · [Open EIPs PR #12097](https://github.com/ethereum/EIPs/pull/12097) · [Discussion](https://ethereum-magicians.org/t/eip-8365-bls-withdrawal-credential-retirement/29284) · [Forkcast](https://forkcast.org/eips/8365)

**Declared prerequisites:** [6110](https://eips.ethereum.org/EIPS/eip-6110), [7251](https://eips.ethereum.org/EIPS/eip-7251), [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md).

**Mechanism.** Gradually queues legacy 0x00 BLS-withdrawal validators for exit under a bounded per-epoch retirement process and standard churn. Rejects new 0x00 validators by ignoring those deposits; existing-validator top-ups continue.

**Relationships.** Requires 6110, 7251, and 7732. Direct prerequisite of optional 8367 balance sunset. This EIP preserves credential-conversion/recovery machinery; it does not alone remove BLS-to-execution changes.

**Ethrex implications.** Primarily CL deposit and exit processing. Ethrex’s deposit extraction must continue reporting deposits accurately, including those the CL may reject.

**Review questions and draft gaps.** Retirement rate is TBD. Distinguish exiting, withdrawal eligibility, and recovery by credential conversion. Validate backlog behavior and user tooling before combining retirement with the separate destructive sunset policy.

### EIP-8367

**Balance sunset for retired BLS validators** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ensi321/EIPs/blob/8ce2c25821fd42d2073fe3268a903772ed868a2e/EIPS/eip-8367.md) · [Open EIPs PR #12099](https://github.com/ethereum/EIPs/pull/12099) · [Discussion](https://ethereum-magicians.org/t/eip-8367-balance-sunset-for-retired-bls-validators/29299) · [Forkcast](https://forkcast.org/eips/8367)

**Declared prerequisites:** [8365](eips.md#eip-8365).

**Mechanism.** After legacy BLS retirement, linearly lowers a balance ceiling from 64 ETH to zero over proposed two to three years and burns 0x00-credential balances above it. Conversion to execution credentials stops further sunset burning.

**Relationships.** Requires 8365. The dependency is one-way: retirement can exist without confiscating remaining balances. Shorter slots 8198 change the wall-clock meaning of epoch schedules.

**Ethrex implications.** CL balance processing and transition fixtures. For ethrex, track integration and validator ecosystem impact rather than treating this as an EL implementation feature.

**Review questions and draft gaps.** Start/end epochs are TBD and assume retirement has drained the relevant active set. This explicitly destroys user balances; evaluate policy and recoverability separately from the small code surface.

### EIP-8375

**ePBS Mandatory Burn of Execution Rewards** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/benaadams/EIPs/blob/b2bb0ef69e4b236703c30468d5773e896410a679/EIPS/eip-8375.md) · [Open EIPs PR #12130](https://github.com/ethereum/EIPs/pull/12130) · [Discussion](https://ethereum-magicians.org/t/eip-8375-ember-epbs-mandatory-burn-of-execution-rewards/29380) · [Forkcast](https://forkcast.org/eips/8375)

**Declared prerequisites:** [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md).

**Mechanism.** Combines universal one-third priority-fee burning with an ePBS bid burn and a public bid floor. For gross bid G, A=floor(G/3), proposer payment P=G-A; burned tips offset the builder’s burn obligation but do not increase P. Requires collateral and PTC-signed floor observations; below-floor branches receive a temporary fork-choice restriction.

**Relationships.** Requires 7732 and changes both EL and CL. Must compose with 8115 batched fee credits, 8141 settlement, shorter-slot deadlines, and FOCIL. Domain 0x0F000000 collides with 8321’s proposed domain and is explicitly provisional.

**Ethrex implications.** Per-transaction cumulative burn accounting, fee-recipient balances, ePBS payments/collateral, Engine integration, and fork-choice integration tests. Self-builds remain subject to priority-fee burning even though exempt from the external-bid floor.

**Review questions and draft gaps.** Numerous floor/timing parameters are TBD. Review off-protocol payment avoidance, liquidity and centralization effects, griefing via public bids, exact wei/gwei rounding, and migration from already active ePBS pending payments; the draft assumes same-fork deployment.


## Frames and inclusion lists

### EIP-7805

**Fork-choice enforced Inclusion Lists (FOCIL)** — Draft; Standards Track; Hegotá: Scheduled; Scheduled headliner.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7805.md) · [Discussion](https://ethereum-magicians.org/t/eip-7805-committee-based-fork-choice-enforced-inclusion-lists-focil/21578) · [Forkcast](https://forkcast.org/eips/7805)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** A committee of 16 validators gossips inclusion lists. Attesters withhold support from a payload omitting a listed transaction that is still valid at the end and fits remaining gas. Inclusion is conditional, with no prescribed transaction position.

**Relationships.** No header requires, but target-fork composition with ePBS (7732) matters. 8369 explores the extension needed to enforce programmable Frames validity; base FOCIL does not imply arbitrary-AA enforcement.

**Ethrex implications.** EL inclusion-list construction, payload updates, post-execution omission checks, and Engine API responses. CL labeling does not mean zero EL work.

**Review questions and draft gaps.** Keep INCLUSION_LIST_UNSATISFIED distinct from an invalid execution payload. Check committee disagreement/equivocation, view deadlines, reorgs, full blocks, and dependency chains in listed transactions. Detailed timing in the base EIP must be checked against the composed ePBS specifications.

### EIP-7906

**Transaction Assertions via State Diff Opcode** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7906.md) · [Discussion](https://ethereum-magicians.org/t/eip-restricted-behavior-transaction-type/23130) · [Forkcast](https://forkcast.org/eips/7906)

**Declared prerequisites:** [2929](https://eips.ethereum.org/EIPS/eip-2929), [8141](eips.md#eip-8141).

**Mechanism.** Adds trailing static POST_TX assertion frames plus TXTRACE, TXDIFF and EVENTDATACOPY to inspect state differences and events. Failed assertions should roll back the application execution suffix while retaining validation/payment effects.

**Relationships.** Requires 2929 and 8141. Interacts with atomic batches, gas refunds, BALs, code changes, and 8298/8151 account migration. Current TXDIFF still permits live-state fallback reads; a proposed narrowing is not the current specification.

**Ethrex implications.** Transaction-local diff/log views, deterministic ordering, additional rollback checkpoint, three opcodes, frame modes and receipt handling.

**Review questions and draft gaps.** Draft prose contradicts itself about rolling back gas payment versus preserving the prefix. Gas costs include TBDs, and opcode allocation is delegated to an 8141 registry not present in the captured base draft. Resolve these before implementation estimates.

### EIP-8141

**Frame Transaction** — Draft; Standards Track; Hegotá: Scheduled; Scheduled.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8141.md) · [Discussion](https://ethereum-magicians.org/t/frame-transaction/27617) · [Forkcast](https://forkcast.org/eips/8141)

**Declared prerequisites:** [1559](https://eips.ethereum.org/EIPS/eip-1559), [2718](https://eips.ethereum.org/EIPS/eip-2718), [2780](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-2780.md), [3529](https://eips.ethereum.org/EIPS/eip-3529), [3607](https://eips.ethereum.org/EIPS/eip-3607), [4844](https://eips.ethereum.org/EIPS/eip-4844), [7594](https://eips.ethereum.org/EIPS/eip-7594), [7623](https://eips.ethereum.org/EIPS/eip-7623), [7702](https://eips.ethereum.org/EIPS/eip-7702), [7708](https://eips.ethereum.org/EIPS/eip-7708), [7778](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7778.md), [7825](https://eips.ethereum.org/EIPS/eip-7825), [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md).

**Mechanism.** Type 0x06 separates programmable validation, payer approval, and execution into frames. Current draft has explicit execution/state gas budgets, native secp256k1/P256 signatures plus arbitrary witnesses, atomic batches, and per-frame receipts.

**Relationships.** Base for 8250, 8272 and 7906. 8369 bridges the validity model to FOCIL. Composition must include 8037 state gas, 7778 block accounting, the 3298 refund change and data floors; AA alone does not supply FOCIL enforcement or quantum safety.

**Ethrex implications.** Substantial existing code and docs target an older revision. Current schema, SIGDATACOPY, gas pools, warming, and policy changes require a version-gap assessment across types, LEVM, mempool, payloads, receipts, RPC and proving.

**Review questions and draft gaps.** Separate consensus validity from public relay policy. A VERIFY failure invalidates the transaction; application failure can remain paid. Audit approval authorization over all later sender frames, prefix DoS, payer exposure, atomic rollback and cross-frame state-gas ownership.

### EIP-8250

**Keyed Nonces for Frame Transactions** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8250.md) · [Discussion](https://ethereum-magicians.org/t/eip-8250-keyed-nonces-for-frame-transactions/28437) · [Forkcast](https://forkcast.org/eips/8250)

**Declared prerequisites:** [7623](https://eips.ethereum.org/EIPS/eip-7623), [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md), [8141](eips.md#eip-8141).

**Mechanism.** Replaces the frame nonce with up to 16 sorted nonce keys and one shared sequence. [0] aliases the account nonce; nonzero keys use NONCE_MANAGER storage. Payment approval atomically consumes all keys and charges first-use state gas.

**Relationships.** Requires 7623, 8037, 8141. Enables replay-independent privacy nullifiers alongside 8272 roots, and is an explicit prerequisite of the 8369 model. Independence of keys does not remove balance/storage conflicts.

**Ethrex implications.** Envelope/signing changes, a fork-installed system contract, atomic nonce bookkeeping, BAL/witness state, introspection, replacement identity and revalidation.

**Review questions and draft gaps.** The public mempool still allows only one pending frame transaction per sender. Its displayed payload flattens fee fields while current 8141 nests them: settle the composed schema. First-use slots cost state gas, and consuming a nullifier survives later paid execution failure.

### EIP-8272

**Recent Roots for Frame Transactions** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8272.md) · [Discussion](https://ethereum-magicians.org/t/eip-8272-recent-roots-for-frame-transactions/28621) · [Forkcast](https://forkcast.org/eips/8272)

**Declared prerequisites:** [7843](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7843.md), [8141](eips.md#eip-8141).

**Mechanism.** Adds a canonical recent-root VERIFY frame carrying up to 16 source/slot/root tuples. A system contract records per-source ring buffers; only roots from earlier slots within the usable window are accepted. No new envelope field or opcode.

**Relationships.** Requires 7843 SLOTNUM and 8141. Complements keyed nonces for private shared-sender transactions, and is referenced by 8369. Root recency is not proof of application correctness.

**Ethrex implications.** System contract activation, canonical frame recognition, narrow validation-trace exceptions, ordinary gas/access effects, root dependency indexing and branch-aware eviction.

**Review questions and draft gaps.** RECENT_ROOT_CODE remains TBD. Reorgs can invalidate many references; expiry and writes need exact slot semantics. Current 8369 still describes envelope roots and skips only expiry during prefix matching, requiring alignment with this revised frame-based design.

### EIP-8369

**VOPS Profiles for FOCIL Eligibility** — Draft; Informational; Hegotá: Proposed; Informational.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8369.md) · [Discussion](https://ethereum-magicians.org/t/eip-8369-vops-profiles-for-focil-eligibility/29298) · [Forkcast](https://forkcast.org/eips/8369)

**Declared prerequisites:** [1559](https://eips.ethereum.org/EIPS/eip-1559), [2718](https://eips.ethereum.org/EIPS/eip-2718), [2930](https://eips.ethereum.org/EIPS/eip-2930), [3607](https://eips.ethereum.org/EIPS/eip-3607), [4844](https://eips.ethereum.org/EIPS/eip-4844), [7702](https://eips.ethereum.org/EIPS/eip-7702), [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md), [7805](eips.md#eip-7805), [7843](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7843.md), [7928](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7928.md), [8141](eips.md#eip-8141), [8250](eips.md#eip-8250), [8272](eips.md#eip-8272).

**Mechanism.** Informational model separating simple end-of-payload validity checks from bounded AA checks against state reconstructed at a builder-claimed index. AA nodes hold code, low account slots, keyed nonces and recent roots; blob transactions are outside both profiles.

**Relationships.** Explicitly references 7805, 8141, 8250, 8272, 7732, 7843 and 7928 plus existing transaction rules. Binding enforcement, claim commitments and tests must be supplied by a future standards-track extension.

**Ethrex implications.** Potential omission replay, BAL-based state reconstruction and Engine API integration. This is an implementation direction, not a complete deployable consensus specification.

**Review questions and draft gaps.** Builder-chosen indices weaken guarantees for transactions whose validity changes within the block. ePBS requires a post-reveal check or disabling this AA profile. Slot counts, budgets, code-byte bounds and the composed Frames encoding remain open. Forkcast excludes it from ranking because it is informational.


## Account abstraction and authorization

### EIP-7819

**SETDELEGATE instruction** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7819.md) · [Discussion](https://ethereum-magicians.org/t/eip-7819-create-delegate/21763) · [Forkcast](https://forkcast.org/eips/7819)

**Declared prerequisites:** [7702](https://eips.ethereum.org/EIPS/eip-7702).

**Mechanism.** SETDELEGATE creates or updates a 7702-style delegation at an address derived from the executing factory and salt. The factory controls upgrades and clearing; this is a protocol-level clone mechanism.

**Relationships.** Requires 7702. Frames mentions it as an optional deployment path, not a hard requirement. Compare with 8298 code reuse and 7851 self-controlled EOA migration; their ownership and lifetime semantics differ.

**Ethrex implications.** New opcode, address derivation, code mutations, nonce initialization, BALs and rollback; execution must observe delegation updates within the same transaction.

**Review questions and draft gaps.** The draft uses old 25000/12500 costs and an existing-account refund; compose with 2780, 8037/8038, and 3298. Multiple code changes, delegation chains, and factory authority need explicit testing.

### EIP-7851

**Code-Controlled EOA Delegation** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7851.md) · [Discussion](https://ethereum-magicians.org/t/eip-7851-code-controlled-eoa-delegation/22344) · [Forkcast](https://forkcast.org/eips/7851)

**Declared prerequisites:** [7702](https://eips.ethereum.org/EIPS/eip-7702).

**Mechanism.** Adds self-only SETSELFDELEGATE and a 0xef0101 delegation prefix. Wallet code can permanently disable the original ECDSA key while retaining the ability to change the delegate through code.

**Relationships.** Requires 7702. 8151 is a companion for contract-level ecRecover authentication. Compare this new-prefix design with 8298's migration to ordinary code; neither is a prerequisite of 8141.

**Ethrex implications.** Delegation resolution, transaction sender rejection, 7702 authorization processing, mempool checks, and journaling. Already-running frames keep their loaded code; later calls resolve the update.

**Review questions and draft gaps.** Current 8151 recognizes empty code and the old 0xef0100 designator, so composing the new prefix changes recovery acceptance. Disabling transaction authority alone does not disable token permits or bespoke signature recovery. Opcode allocation remains TBD.

### EIP-8151

**Account Code Restricted ecRecover** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8151.md) · [Discussion](https://ethereum-magicians.org/t/eip-8151-account-code-restricted-ecrecover/27690) · [Forkcast](https://forkcast.org/eips/8151)

**Declared prerequisites:** [2929](https://eips.ethereum.org/EIPS/eip-2929), [3607](https://eips.ethereum.org/EIPS/eip-3607), [7702](https://eips.ethereum.org/EIPS/eip-7702).

**Mechanism.** Makes ecRecover inspect the recovered account’s raw code: accept empty code or exact EIP-7702 EOA delegation; otherwise return a zero word. Adds warm/cold account-access charging on top of the 3000 base cost.

**Relationships.** Requires 2929, 3607, and 7702. Complements 7851 and 8298 by disabling this legacy contract-authentication path after migration. 8141 supports alternate authentication, but does not itself change ecRecover.

**Ethrex implications.** The precompile becomes state-dependent: pass account access into dispatch, record warming/BAL/witness reads, and preserve historical behavior. L2 proofs cannot blindly substitute an L1 ecRecover call when the account states differ.

**Review questions and draft gaps.** The draft describes failed recovery as an existing 32-byte-zero result; ethrex currently returns empty bytes on malformed/failed recovery. Resolve that compatibility discrepancy. Custom recovery code remains possible, so this is not a universal revocation of ECDSA.

### EIP-8298

**SETCODEFROM Code Reuse Instruction** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8298.md) · [Discussion](https://ethereum-magicians.org/t/eip-8298-setcodefrom-code-reuse-instruction/28779) · [Forkcast](https://forkcast.org/eips/8298)

**Declared prerequisites:** [3529](https://eips.ethereum.org/EIPS/eip-3529), [3607](https://eips.ethereum.org/EIPS/eip-3607), [6780](https://eips.ethereum.org/EIPS/eip-6780), [7702](https://eips.ethereum.org/EIPS/eip-7702), [8037](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8037.md), [8038](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8038.md).

**Mechanism.** Adds SETCODEFROM to replace the current execution-context account’s code with an existing account’s regular code. The running frame keeps its original code; later calls use the replacement. No initcode execution or new-code deposit is involved; static mode and empty/precompile/delegation sources are disallowed.

**Relationships.** Requires 3529, 3607, 6780, 7702, 8037, and 8038. Complements 8151 for disabling legacy signature authorization, and offers another migration mechanism alongside 7851.

**Ethrex implications.** Opcode execution, immutable code-by-hash lifetime, account journals, authorization, warm/cold accesses, refunds, state gas, and witnesses. Code must remain retrievable after the source account later changes.

**Review questions and draft gaps.** Opcode allocation is unfinished. Reconcile its account-write cost with 8358; audit delegatecall execution contexts, rollback, and cases where code changes while frames or cached code objects remain active.


## Data availability, proofs, and synchronization

### EIP-8025

**Optional Execution Proofs** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8025.md) · [Discussion](https://ethereum-magicians.org/t/eip-optional-execution-proofs/25500) · [Forkcast](https://forkcast.org/eips/8025)

**Declared prerequisites:** [4844](https://eips.ethereum.org/EIPS/eip-4844), [6110](https://eips.ethereum.org/EIPS/eip-6110), [7002](https://eips.ethereum.org/EIPS/eip-7002), [7251](https://eips.ethereum.org/EIPS/eip-7251), [7688](https://eips.ethereum.org/EIPS/eip-7688), [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md), [7928](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7928.md), [8282](https://eips.ethereum.org/EIPS/eip-8282).

**Mechanism.** Adds optional, supplementary execution proofs, with validator-associated provers, a registry of proof types, proof gossip, and a stateless execution interface. The current normative design still requires verifying nodes to re-execute; absent proofs do not delay attestations or alter fork choice. Proofs are bounded to 400 KiB each and at most four types.

**Relationships.** Requires the execution/request/BAL/ePBS foundations listed in the metadata, including 7928 and 8282. 8142 could support future proof-oriented data availability, but is not a declared prerequisite. Changes to every enabled EVM feature affect the statement being proven.

**Ethrex implications.** Direct overlap with ethrex stateless execution, execution witnesses, SSZ inputs, and SP1/RISC0/OpenVM/ZisK prover backends. Guest validation must bind the payload request, chain ID, schema, and success result exactly to CL verification.

**Review questions and draft gaps.** The EIP summary and its pinned normative sources disagree on public-input schemas: the EL result carries chain_config while the EIP describes chain_id/schema_id, and CL PublicInput exposes only the request root. The pinned guest targets Amsterdam. Resolve these interfaces and Hegotá fork coverage before claiming conformance; proof-registry parameters remain provisional.

### EIP-8142

**Block-in-Blobs (BiB)** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8142.md) · [Discussion](https://ethereum-magicians.org/t/eip-8142-block-in-blobs-bib/27621) · [Forkcast](https://forkcast.org/eips/8142)

**Declared prerequisites:** [4844](https://eips.ethereum.org/EIPS/eip-4844), [7594](https://eips.ethereum.org/EIPS/eip-7594), [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md), [7892](https://eips.ethereum.org/EIPS/eip-7892), [7928](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7928.md).

**Mechanism.** Encodes canonical transactions and the BAL into payload blobs using 31 bytes per field element. KZG commitments bind these blobs to the payload, and a new payload_blob_count separates payload blobs from user blobs.

**Relationships.** Requires 4844, 7594, 7732, 7892, and 7928. Complementary to optional proofs 8025 and BAL sidecars 8146; neither relationship is a declared dependency in this EIP.

**Ethrex implications.** Payload building/validation, header and Engine fields, blob construction/KZG work, bandwidth/capacity accounting, and stateless proof inputs. Payload blobs share the available blob capacity; their cost is not automatically charged to a particular user.

**Review questions and draft gaps.** Commitment consistency and data availability are different checks. Model contention with user blobs, failure recovery, and the cost of reconstructing full payloads under the proposed network design.

### EIP-8146

**Block Access List Sidecars** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8146.md) · [Discussion](https://ethereum-magicians.org/t/eip-8146-block-access-list-sidecars/27757) · [Forkcast](https://forkcast.org/eips/8146)

**Declared prerequisites:** [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md), [7928](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7928.md).

**Mechanism.** Detaches BAL bytes from the execution payload into a sidecar committed by the bid. engine_notifyBlockAccessListV1 permits early delivery and prefetch. The current normative newPayload flow requires the matching BAL before validation can proceed.

**Relationships.** Requires 7732 and 7928. Shares payload-data infrastructure with 8142 and execution/proof inputs with 8025. Earlier delivery is a scheduling opportunity, not proof that the advertised accesses are correct.

**Ethrex implications.** Engine methods/versioning, sidecar cache and retention, prefetch scheduling, payload validation, and bounded unsolicited data handling. The draft specifies an 8 MiB bound and 3533-epoch retention.

**Review questions and draft gaps.** The current draft’s wait-for-BAL rule differs from proposals to make validation independent of its arrival. Evaluate the deadline/fallback behavior alongside ePBS, FOCIL, and shorter slots.

### EIP-8237

**Independent CL/EL Sync** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8237.md) · [Discussion](https://ethereum-magicians.org/t/eip-8237-independent-cl-el-sync/28331) · [Forkcast](https://forkcast.org/eips/8237)

**Declared prerequisites:** [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md).

**Mechanism.** Enables independent CL and EL synchronization using a partial_header_hash that commits the execution-header fields needed by consensus, including parent-chain linkage. Adds payload/header and Engine support so the CL can verify its chain view before full EL execution is available.

**Relationships.** Requires 7732. Compare 8379, which has the CL feed historical payloads through Engine while the EL acquires state. These are different synchronization architectures, not an established mutually exclusive pair.

**Ethrex implications.** Header construction/validation, payload bid fields, Engine interfaces, sync progress/divergence detection, and persistence. The eventual EL result must still agree with the committed header.

**Review questions and draft gaps.** Specify recovery when independently synced views diverge, and distinguish incomplete execution from invalid execution. Evaluate with partial bids 8341 and delayed roots 7862 before combining header commitments.

### EIP-8341

**Partial Execution Payload Commitments** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/nerolation/EIPs/blob/7c0a835f2be7a1678990473a20212fb97e3ed017/EIPS/eip-8341.md) · [Open EIPs PR #11936](https://github.com/ethereum/EIPs/pull/11936) · [Discussion](https://ethereum-magicians.org/t/eip-8341-partial-execution-payload-commitments/29030) · [Forkcast](https://forkcast.org/eips/8341)

**Declared prerequisites:** [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md).

**Mechanism.** Changes ePBS bids to commit payload fields except state_root and block_hash. The builder can finish execution after bidding; the revealed payload must still execute correctly and match its full parent hash.

**Relationships.** Requires 7732. An alternative way to remove execution from the bid deadline compared with delayed state roots 7862. Current text explicitly requires no EL or Engine changes.

**Ethrex implications.** Most protocol work is CL bid/fork-choice logic. An ethrex builder may need to separate provisional payload preparation from execution and sealing to exploit the benefit; do not label that as a mandated new Engine API.

**Review questions and draft gaps.** Check equivocation/seen-envelope handling so an invalid early root cannot suppress a later valid envelope. The text assumes deployment with ePBS; later activation needs a defined transition.

### EIP-8371

**RowDAS - Distributed Blob Reconstruction** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8371.md) · [Discussion](https://ethereum-magicians.org/t/eip-8371-rowdas-distributed-blobspace-reconstruction/29320) · [Forkcast](https://forkcast.org/eips/8371)

**Declared prerequisites:** [7594](https://eips.ethereum.org/EIPS/eip-7594), [8136](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8136.md).

**Mechanism.** RowDAS accelerates blob reconstruction with 128 row gossip subnets and partial messages. Nodes collect enough verified cells (64 of 128) to reconstruct assigned rows and propagate missing pieces, with phased delays and supernode fallback.

**Relationships.** Requires 7594 and 8136. Related to the earlier unnumbered Partial Reconstruction and 2D PeerDAS headliner discussion, but should not be treated as an identical specification. It preserves the underlying commitment/sampling scheme.

**Ethrex implications.** Mainly CL blob/cell networking. Ethrex sees resulting sidecar availability and timing; coordinate KZG and blob transport assumptions with 8094/8142.

**Review questions and draft gaps.** Wire details and delay constants remain incomplete. Test reconstruction CPU, equivocation bounds, metadata feedback attacks, eclipse behavior, and fallback reliability; reconstruction speed does not itself establish stronger availability guarantees.

### EIP-8379

**Top-up Sync** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8379.md) · [Discussion](https://ethereum-magicians.org/t/eip-8379-top-up-sync/29405) · [Forkcast](https://forkcast.org/eips/8379)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Lets the CL supply canonical historical execution payloads through Engine while the EL independently acquires state. Adds engine_getSyncStatus and distinguishes MISSING_STATE from missing-ancestor SYNCING, with separate block and executed-state heads.

**Relationships.** No declared prerequisites. Compare 8237 independent dual-layer synchronization; overlapping goals do not imply equivalent machinery or a formal conflict.

**Ethrex implications.** Engine versioning across historical forks, persistent unexecuted blocks, forkchoice-driven canonicalization, state/snap synchronization, reorg recovery, and bounded buffering.

**Review questions and draft gaps.** The proposed stable-head semantics between Engine calls constrain asynchronous state updates. Define resource bounds, restart behavior, and status transitions; this is more than adding a progress getter.

### EIP-8383

**Reduce CL Block Retention Window** — Draft; Informational; Hegotá: Proposed; Informational.

[Pinned specification](https://github.com/kevaundray/EIPs/blob/9f7c34af6c079e225cec7fc19b715a5731e2969d/EIPS/eip-8383.md) · [Open EIPs PR #12188](https://github.com/ethereum/EIPs/pull/12188) · [Discussion](https://ethereum-magicians.org/t/eip-8383-reduce-cl-block-retention-window/29449) · [Forkcast](https://forkcast.org/eips/8383)

**Declared prerequisites:** None in the EIP header.

**Mechanism.** Informational guidance to reduce CL block-serving retention from 33024 to 8192 epochs, about 36 days at 12-second slots. It argues that weak-subjectivity requirements can fit within the shorter period.

**Relationships.** No declared prerequisites. Related to checkpoint sync and 8379; recalculate the reasoning if 8198 changes slot duration or churn parameters change. It does not authorize dropping execution history.

**Ethrex implications.** Primarily CL storage/network defaults and checkpoint availability. Keep distinct from ethrex EL history expiry and state retention.

**Review questions and draft gaps.** Excluded by Forkcast’s Informational filter. Verify the weak-subjectivity calculation against the actual fork parameters and ensure viable checkpoint/bootstrap sources; the proposal is not a consensus hard fork.


## Mempool and transaction propagation

### EIP-8077

**eth/XX - announce transactions with nonce** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8077.md) · [Discussion](https://ethereum-magicians.org/t/eip-8077-eth-xx-add-nonce-and-source-to-transactions-announcement/26505) · [Forkcast](https://forkcast.org/eips/8077)

**Declared prerequisites:** [7642](https://eips.ethereum.org/EIPS/eip-7642).

**Mechanism.** Extends pooled-transaction announcements with sender and nonce, letting peers suppress redundant or obsolete fetches before downloading bodies. These claims are untrusted until transaction validation.

**Relationships.** Requires 7642. Complementary to 8094 blob retrieval, not dependent on it. Keyed nonces in 8250 create an additional metadata design question.

**Ethrex implications.** eth capability negotiation, announcement encoding, pool lookup/replacement, validation of claimed metadata, and peer accounting. It can be rolled out as a networking capability rather than a consensus hard fork.

**Review questions and draft gaps.** Protocol version remains a placeholder. Avoid letting a forged sender/nonce announcement permanently suppress a valid transaction; specify metadata for every supported transaction type.

### EIP-8094

**eth/vhash - Blob-Aware Mempool** — Draft; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8094.md) · [Discussion](https://ethereum-magicians.org/t/eip-8094-eth-vhash-blob-aware-mempool/26834) · [Forkcast](https://forkcast.org/eips/8094)

**Declared prerequisites:** [7642](https://eips.ethereum.org/EIPS/eip-7642).

**Mechanism.** Separates blob transaction propagation from sidecar retrieval. Peers can reuse already held blobs by versioned hash, fetch missing blobs through new messages, and forward transactions only after required sidecars are verified.

**Relationships.** Requires 7642. Pairs naturally with 8077 but neither formally requires the other. Shares blob transport machinery with 8142 and blob-bearing 8141; type-6 support is not automatically supplied by a type-3 design.

**Ethrex implications.** P2P message/version changes, sidecar storage and deduplication, pool admission, bounded pending-fetch state, and KZG validation. Replacement transactions can reuse blob data.

**Review questions and draft gaps.** Message codes and some wire details are unfinished. Account for missing/invalid blobs, request amplification, eviction, and sparse-blob mempool proposals before treating the bandwidth savings as guaranteed.


## Privacy and encrypted ordering

### EIP-8105

**Universal Enshrined Encrypted Mempool** — Draft; Standards Track; Hegotá: Withdrawn; Withdrawn.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8105.md) · [Discussion](https://ethereum-magicians.org/t/eip-8105-universal-enshrined-encrypted-mempool/27201) · [Forkcast](https://forkcast.org/eips/8105)

**Declared prerequisites:** [7732](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-7732.md).

**Mechanism.** Withdrawn Hegotá proposal for a scheme-agnostic encrypted mempool. An EL registry defines decryption/key-validation providers and a directed trust graph. Plaintext envelopes pay gas first; decrypted transactions execute in order in the following block if keys are available and the transaction is valid. PTC votes record key availability.

**Relationships.** Requires 7732. Historical predecessor/alternative to 8184. Provides temporary concealment before ordering, whereas 8182 targets persistent transfer privacy. Draft type 0x06 collides with scheduled Frame transactions and must not be reused unchanged.

**Ethrex implications.** Would require new transaction types, cross-block execution/gas obligations, registry/request integration, PTC/Engine changes, and key gossip. Include for historical context, not as a current rankable item.

**Review questions and draft gaps.** Registry code is TBD; providers can withhold keys after charging envelopes. Trust-graph ordering, strategic withholding, doubled adjacent-block work, and missing-key behavior are central open design issues.

### EIP-8182

**Private ETH and ERC-20 Transfers** — Review; Standards Track; Hegotá: Proposed; Rankable.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8182.md) · [Discussion](https://ethereum-magicians.org/t/eip-8182-private-eth-and-erc-20-transfers/27889) · [Forkcast](https://forkcast.org/eips/8182)

**Declared prerequisites:** [20](https://eips.ethereum.org/EIPS/eip-20).

**Mechanism.** Installs a canonical shielded pool for ETH and compatible ERC-20s. A fork-managed Groth16 BN254 proof enforces note membership, conservation, nullifiers, and intent binding; a separate user-selected verifier checks authorization. Deposits are proof-free; spends use two input slots and three output slots. No new opcode, transaction type, or precompile is required.

**Relationships.** Requires ERC-20 only. Encrypted ordering proposals 8105/8184 address a different privacy boundary and are optional complements. Alternate authentication does not make the canonical pool proof quantum-safe.

**Ethrex implications.** Protocol-managed contract deployment and future upgrades, exact cryptographic assets/verification keys, activation state, execution/prover performance, and interoperability vectors. Wallet note delivery is explicitly outside the proposal.

**Review questions and draft gaps.** Trusted setup and circuit audits are central. Deposits/withdrawals remain public; root windows can expire during congestion, policy revocation is delayed up to 64 blocks, note/nullifier state accumulates, and opaque delivery data can prevent recovery. System-contract scope does not make this a trivial change.

### EIP-8184

**LUCID encrypted mempool** — Draft; Standards Track; Hegotá: Declined; Declined.

[Pinned specification](https://github.com/ethereum/EIPs/blob/991d932f52a56477753cd9f62114b842cd77275c/EIPS/eip-8184.md) · [Discussion](https://ethereum-magicians.org/t/eip-8184-lucid-encrypted-mempool/28017) · [Forkcast](https://forkcast.org/eips/8184)

**Declared prerequisites:** [2718](https://eips.ethereum.org/EIPS/eip-2718), [7805](eips.md#eip-7805).

**Mechanism.** LUCID commits sealed-transaction tickets through inclusion lists/bids, reveals keys after commitment, and executes decrypted transactions at the top of the next payload. Tickets pay fees and consume nonces; a bounded top-of-block gas reservation limits deferred work. PTC and attesters track ciphertext/key availability.

**Relationships.** Declined for Hegotá; requires 2718 and 7805 in metadata. Builds on ePBS structures in the body. 8141 integration is an explicitly future coordinated extension using new frame modes, not current compatibility. Related to withdrawn 8105.

**Ethrex implications.** Would touch transaction envelopes, pool propagation, Engine payload/bid data, cross-block gas and fee obligations, inclusion-list checks, key messages, and reorg bookkeeping.

**Review questions and draft gaps.** Keep outside the active ranking roster. Analyze strategic withholding, unavailable data, failed decryption, paid tickets that do not execute, and FOCIL/ePBS timing. Draft constants and alternative trust-graph extensions are not settled protocol parameters.


# Ethrex implementation context

[Overview](README.md) · [Proposal profiles](eips.md) · [Dependencies](dependencies.md)

Source inspection targets ethrex commit `43540d8deb19a985a776e7c8a19a78428827233c`. The observations below identify existing machinery and places affected by candidate EIPs. They are not a full implementation audit, conformance result, or effort estimate. No client code was changed or benchmarked for this research.

## Frames: substantial existing work, older specification

[docs/eip-8141.md](../../eip-8141.md) explicitly targets EIPs commit `c55786f42` from 2026-07-28. It describes real code across transaction encoding, execution, validation-prefix simulation, paymaster reservations, receipts, RPC, and broadcasting. The current research snapshot of [8141](eips.md#eip-8141) is later and differs in consensus-visible ways.

| Interface | Inspected ethrex code | Captured current 8141 |
| --- | --- | --- |
| Frame limits | `Frame.gas_limit: u64`; scalar RLP field | `limits = [execution, state]`; independently declared budgets |
| Outer fee encoding | Three fee fields encoded directly into the transaction list | A nested `fees` list, giving a seven-field outer payload |
| Base intrinsic cost | `FRAME_TX_INTRINSIC_COST = 15000` | 12000; per-frame overhead remains 475 |
| Frame receipt gas | `FrameReceipt.gas_used: u64` | A two-element execution/state value, with journaled cross-frame state-charge attribution |
| Arbitrary signature bytes | `SIGPARAM` parameter 0x04 performs the copy | Dedicated `SIGDATACOPY` at 0xb5 |
| Hegotá opcode table | Installs APPROVE and 0xb0–0xb4 | Also needs SIGDATACOPY at 0xb5 |
| Public validation budget | Existing simulation checks `total_gas_used` against 100000 | Requires distinct execution and state bounds, including `MAX_VERIFY_STATE_GAS = 500000` |
| Blob-bearing Frames | Pool admission returns `FrameTxBlobsUnsupported` | Consensus envelope supports blobs; relay/build transport requires its own integration |

Evidence: [transaction types and RLP](../../../crates/common/types/transaction.rs), [frame receipts](../../../crates/common/types/receipt.rs), [frame opcode handlers](../../../crates/vm/levm/src/opcode_handlers/frame_tx.rs), [fork opcode tables](../../../crates/vm/levm/src/opcodes.rs), [validation simulation](../../../crates/vm/backends/levm/mod.rs), and [pool admission](../../../crates/blockchain/blockchain.rs). Search the named symbols rather than relying on line numbers after future edits.

The blob admission comment also explains that import accounts for Frame blobs while the build path does not yet add them to header blob gas. Lifting that gate requires transport and builder accounting together. Existing P2P [broadcasting](../../../crates/networking/p2p/tx_broadcaster.rs) announces Frame hashes rather than pushing full transactions to selected peers.

Before treating Frames as complete against a new target, reconcile the wire/signing schema, independent gas pools, state-charge ownership/refills, receipts, introspection, validation restrictions, and payer exposure as one versioned change. Reuse existing implementation and tests, but do not infer current conformance from their presence. The [fork roadmap](../../roadmaps/forks-roadmap.md) still describes 8141 as CFI; the captured Forkcast feed and meta EIP now mark it Scheduled.

## Where proposal families meet the code

| Surface | Existing entry points | Proposals and likely work |
| --- | --- | --- |
| Transaction identity and encoding | [transaction.rs](../../../crates/common/types/transaction.rs), [RPC types](../../../crates/networking/rpc/types) | 8141/8250 affect envelopes, signature hashes, field validation, replacement identity, and RPC serialization. 8077 must announce useful nonce metadata for supported types. |
| EVM instructions and control flow | [opcode handlers](../../../crates/vm/levm/src/opcode_handlers), [opcodes.rs](../../../crates/vm/levm/src/opcodes.rs), [call_frame.rs](../../../crates/vm/levm/src/call_frame.rs) | 2488, 4758, 5920, 7645, 7819, 7851, 7906, 7979, 8219, and 8298 change dispatch, call context, rollback, or code lifetime. Historical fork behavior remains necessary. |
| Gas and memory | [gas_cost.rs](../../../crates/vm/levm/src/gas_cost.rs), [memory.rs](../../../crates/vm/levm/src/memory.rs), [vm.rs](../../../crates/vm/levm/src/vm.rs), [default hooks](../../../crates/vm/levm/src/hooks/default_hook.rs) | 3298, 7923, 8131, 8279, 8358, 8368, 8372, and 8374 span metering, state baselines, reverts, floors, refunds, and resource reservation. A changed constant can require changes beyond its definition. |
| Cryptography and account reads | [precompiles.rs](../../../crates/vm/levm/src/precompiles.rs) | 7666/8200 move execution into bytecode; 8355 adds ML-DSA verification; 8151 makes recovery depend on the recovered account’s state and access status. |
| Pool validation and revalidation | [mempool.rs](../../../crates/blockchain/mempool.rs), [blockchain.rs](../../../crates/blockchain/blockchain.rs), [validation_observer.rs](../../../crates/vm/levm/src/validation_observer.rs), [LEVM backend](../../../crates/vm/backends/levm/mod.rs) | Frames companions affect prefix recognition, sender/key identity, root expiry, dependency invalidation, replacement, and sponsor reservations. Networking proposals add separate fetching and sidecar state. |
| Payload construction and import | [payload.rs](../../../crates/blockchain/payload.rs), [blockchain.rs](../../../crates/blockchain/blockchain.rs), [prewarm.rs](../../../crates/blockchain/prewarm.rs) | FOCIL omission checks, 8115 fee timing, 8142 payload blobs, 8146 BAL arrival, 8375 fee burns, and 7862 root scheduling cross construction/validation boundaries. |
| Engine integration and synchronization | [Engine handlers](../../../crates/networking/rpc/engine), [fork choice](../../../crates/blockchain/fork_choice.rs), [block buffers](../../../crates/storage/block_data_buffer.rs) | 7805, 8142, 8146, 8237, and 8379 need coordinated EL/CL interfaces. 8341 currently claims no required Engine change; builder scheduling is a separate implementation opportunity. |
| Headers, receipts, and BALs | [block.rs](../../../crates/common/types/block.rs), [receipt.rs](../../../crates/common/types/receipt.rs), [block_access_list.rs](../../../crates/common/types/block_access_list.rs) | 7668, 7807, 7862, 8116, 8141, 8142, 8237, and 8304 affect different commitments/encodings. Coordinate network, RPC, persistence, and proof consumers. |
| Account/storage representation | [account.rs](../../../crates/common/types/account.rs), [storage](../../../crates/storage), [trie](../../../crates/common/trie) | 8188 adds write metadata to consensus encodings, while 8253 performs a narrow activation mutation. 8298 affects code-hash lifetime and retrieval. These have very different migration costs. |
| System contracts and requests | [system_contracts.rs](../../../crates/vm/system_contracts.rs), [requests.rs](../../../crates/common/types/requests.rs) | 8148/8205 add request machinery; 8250/8272 add validation-related state/contracts; 8182 adds a protocol-managed pool; 8304 adds an index. Include deployment, system ordering, gas, BALs, and witnesses. |
| Stateless validation and proving | [stateless.rs](../../../crates/blockchain/stateless.rs), [stateless_ssz.rs](../../../crates/common/types/stateless_ssz.rs), [block_execution_witness.rs](../../../crates/common/types/block_execution_witness.rs), [prover backends](../../../crates/prover/src/backend) | 8025 is a direct connection, but every selected consensus change alters the guest’s execution statement or witness requirements. Native verifier throughput and prover cost are separate measurements. |

## Three concrete baseline observations

**Gas parameters are already fork-specific.** `cost_per_state_byte` in [gas_cost.rs](../../../crates/vm/levm/src/gas_cost.rs) returns a fixed 1530 and explicitly says the dynamic formula is inactive. Compare 8368/8372 to the actual baseline, then follow changes through transaction admission, execution/state reservations, header gas, estimates, and proof execution.

**Account encoding is still the familiar four-field representation.** [AccountState](../../../crates/common/types/account.rs) holds nonce, balance, storage root, and code hash; its RLP encoder writes those four fields. EIP-8188 therefore reaches into consensus state encoding and mixed-version decoding, not just an extra database column. Slot encoding, actual-write detection, and journal restoration must follow the same rule.

**ecRecover currently has no account-state input.** The function in [precompiles.rs](../../../crates/vm/levm/src/precompiles.rs) accepts calldata, gas, fork, and a crypto provider. Invalid recovery returns empty bytes. EIP-8151 requires state lookup/warming and describes a zero-word failure result; both the interface and that semantic discrepancy need explicit treatment. A proof system that delegates recovery to another chain’s precompile must preserve the account-state meaning.

## Evidence to request when evaluating a candidate

For a proposal under active consideration, pin the composed fork rules first. Then ask for evidence that addresses its actual uncertainty:

- **Encoding/state changes:** cross-client vectors, activation boundaries, mixed historical/new decoding, state roots, reorgs, and witness round trips.
- **Gas/refund changes:** adversarial workloads, original/current-value transitions, nested reverts, out-of-gas boundaries, estimation accuracy, and block packing under both resource dimensions.
- **Frames/FOCIL:** separately verify consensus validity, public relay admissibility, omission eligibility, payer exposure, and reorg invalidation. Reusing one check for all five can be incorrect.
- **Networking/timing:** missing and invalid data, mixed-capability peers, bounded caches, actual tail latency, and recovery under ePBS/FOCIL deadlines and shorter slots.
- **Cryptography/proving:** canonical input validation, independent vectors, worst-case native execution, complete guest support, and end-to-end proof time/memory on declared hardware.
- **CL-only changes:** identify the CL implementation and integration obligations rather than estimating imaginary LEVM work. Economic proposals additionally need explicit behavioral/security models.

This reference validates research coverage and source consistency only. Client tests and performance runs should accompany actual implementation or a concrete empirical question; they were not run merely to substantiate these documentation changes.

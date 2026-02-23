# Native Rollups: Gap Analysis

Compares our EXECUTE precompile PoC against the [L2Beat native rollups book](https://native-rollups.l2beat.com/) ([EIP-8079](https://github.com/ethereum/EIPs/pull/9608)).

---

## EXECUTE Precompile

The book defines two variants: `apply_body` (individual fields, skips header validation) and `state_transition` (full headers, complete STF). We implement `apply_body`.

The book's `apply_body` processing includes a step we don't have: **process unchecked system transaction for L1 anchoring** (step 4 of 8). The book also shows a `sequence()`/`settle()` split in its usage example; we combine both in `advance()`.

| Input | Book | Us | Notes |
|-------|------|-----|-------|
| `chain_id` | Explicit parameter | From witness chain config | Minor gap |
| `number` | Yes | Yes (`blockNumber`) | |
| `pre_state` / `post_state` | Yes | Yes | |
| `post_receipts` | Yes | Yes | |
| `block_gas_limit` | Yes | Yes (on-chain) | |
| `coinbase` | Yes | Yes | |
| `prev_randao` | Yes | Yes | |
| `timestamp` | TBD (sequencing vs proving) | Explicit input | |
| `transactions` | Blob references (TBD) | RLP list in calldata | Different serialization |
| `parent_gas_limit/used/base_fee` | Yes | Yes (on-chain) | |
| `l1_anchor` | Arbitrary `bytes32` | Merkle root of L1 messages | Narrower scope (messages only, not generic) |
| `parent_beacon_block_root` | TBD, configurable | Not included | Minor gap |
| System transaction | Step 4 of `apply_body` | L1Anchor predeploy write before tx execution | **Aligned** |

**Output:** Book says TBD (possibly `block_gas_used`). We return 160 bytes: `(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas)`.

---

## L1 Anchoring

The book proposes an `L1_ANCHOR` system contract on L2 that receives an arbitrary `bytes32` via a system transaction. Format-agnostic — can be an L1 block hash, messages root, anything. Higher-level messaging built on top via inclusion proofs.

We implement the L1Anchor predeploy at `0x00...fffe` with a single `bytes32 public l1MessagesRoot` at storage slot 0. The EXECUTE precompile writes the `l1Anchor` (a Merkle root over consumed L1 message hashes) directly to this storage slot before executing regular transactions — matching step 4 of `apply_body`. The L2Bridge reads from L1Anchor to verify Merkle inclusion proofs for individual L1 messages.

Remaining gap: our `l1Anchor` is specifically an L1 messages Merkle root, not a generic `bytes32` (e.g., L1 block hash). A production implementation could anchor an L1 block hash instead for broader cross-chain proofs.

---

## L1→L2 Messaging

Book's principle: **no custom transaction types**. Messages claimed via inclusion proofs against anchored hashes (Linea/Taiko style).

We implement this proof-based approach. The L1 NativeRollup contract computes a Merkle root over consumed L1 message hashes and passes it to EXECUTE as the `l1Anchor`. The EXECUTE precompile writes this root to the L1Anchor predeploy. On L2, a relayer sends regular signed txs calling `L2Bridge.processL1Message()` with Merkle inclusion proofs against the anchored root. The state root check at the end of EXECUTE implicitly guarantees correct message processing. Supports arbitrary calldata. The relayer pays gas, solving the **first deposit problem** (which the book lists as unresolved).

Aligned with the book's recommended Linea/Taiko-style proof-based messaging.

---

## L2→L1 Messaging (Withdrawals)

Book is uncertain — suggests state root or receipts root proofs once statelessness reduces proof costs. Spec is TBD.

We use state root proofs, aligned with the book's recommendation. The L2Bridge writes `sentMessages[withdrawalHash] = true` to storage when users withdraw. The EXECUTE precompile returns the post-state root (which captures L2Bridge storage), and NativeRollup stores it in `stateRootHistory[blockNumber]`. Users claim on L1 with MPT account proof (state root → L2Bridge storageRoot) and storage proof (storageRoot → `sentMessages[hash] == true`). Similar to the OP Stack's `L2ToL1MessagePasser` pattern. The MPT verification is inlined in the NativeRollup contract. No finality delay.

---

## Gas Token Deposits

Book recommends preminted tokens in an L2 predeploy (Linea/Taiko pattern). Supports arbitrary gas tokens (ERC20, NFTs).

We implement the preminted ETH approach. ETH only — no custom gas tokens.

---

## L2 Fee Market

Book proposes exposing `burned_fees` in `block_output` so the L1 bridge can credit them. DA cost pricing is WIP.

We compute `burnedFees = base_fee_per_gas * block_gas_used`. This is equivalent to the book's per-tx formula (`effective_gas_fee - gas_refund_amount - transaction_fee + blob_gas_fee`) — it reduces to `gas_used * base_fee` since `gas_used` already accounts for EIP-3529 refunds and we reject blob txs (`blob_gas_fee = 0`). The L1 contract sends burned fees to the relayer. L2-side crediting (`BaseFeeVault` pattern) is not implemented.

---

## Summary Table

| Aspect | Book | Us | Alignment |
|--------|------|-----|-----------|
| `apply_body` variant | Specified | Implemented | **Aligned** |
| State root validation | Required | Implemented | **Aligned** |
| Receipts root validation | Required | Implemented | **Aligned** |
| Base fee from parent params | Required | EIP-1559 computation | **Aligned** |
| Parent gas tracking on L1 | Implied | On-chain storage | **Aligned** |
| Coinbase as input | Required | Implemented | **Aligned** |
| Burned fees in output | Proposed | Implemented | **Aligned** |
| Tx filtering (blobs) | Required | Implemented (+ ethrex types) | **Aligned+** |
| Statelessness | Required (EIP-7864) | `ExecutionWitness`-based | **Aligned** |
| `prev_randao` configurable | Required | Implemented | **Aligned** |
| Preminted gas tokens | Recommended | Implemented | **Aligned** |
| No custom tx types | Design principle | Achieved (relayer txs) | **Aligned** |
| First deposit problem | Unresolved in book | Solved (relayer pays gas) | **Ahead** |
| `chain_id` as input | Explicit parameter | From witness chain config | **Minor gap** |
| `parent_beacon_block_root` | TBD, configurable | Not included | **Minor gap** |
| System transaction | Step in `apply_body` | L1Anchor predeploy write | **Aligned** |
| L1 anchoring (`L1_ANCHOR`) | System contract + system tx | L1Anchor predeploy + system write | **Aligned** |
| L1→L2 messaging | Proof-based (no custom tx) | Merkle proofs against anchored root | **Aligned** |
| L2→L1 messaging | State/receipts root proofs (TBD) | State root proofs (MPT account + storage) | **Aligned** |
| Forced transactions | WIP (FOCIL, threshold) | Not implemented | **Gap** |
| Gas metering | TBD | Flat 100k gas | **Both TBD** |
| Serialization | Blob references (TBD) | RLP calldata + JSON witness | **Different** |
| EXECUTE output format | TBD | 160 bytes (5 fields) | **We defined** |
| L2-side burned fee handling | Not specified | Not implemented | **Gap** |
| Finality delay | Implied for production | Not implemented | **Gap** |
| DA cost pricing | WIP | Not implemented | **Both WIP** |
| Custom gas tokens | Supported (ERC20, NFTs) | ETH only | **Partial** |

---

## Remaining Gaps

### Addressable (PoC scope)

| Gap | Description | Effort |
|-----|-------------|--------|
| Finality delay | Add `FINALITY_DELAY` to `claimWithdrawal()` | Low |
| `chain_id` input | Pass as explicit EXECUTE parameter | Low |

### Future Work (beyond PoC)

| Gap | Why it's future work |
|-----|----------------------|
| Generic L1 block hash anchoring | L1Anchor currently stores L1 messages Merkle root; generic L1 block hash would enable broader cross-chain proofs |
| Forced transactions | Book's design is WIP/brainstorming |
| Blob-referenced transactions | Requires blob DA infrastructure |
| Gas metering | Spec is TBD |
| L2-side burned fee handling | `BaseFeeVault`-style redirect to prevent ETH supply drain |
| Custom gas tokens | Out of scope for ETH-only PoC |
| DA cost pricing | Book is WIP |

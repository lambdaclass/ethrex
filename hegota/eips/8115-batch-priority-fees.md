# EIP-8115: Batch priority fees at end of block
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/242, 2026-07-30 (champion: Etan Kissling)
- **Authors:** Etan Kissling, Gajinder Singh
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8115) · [discussion](https://ethereum-magicians.org/t/eip-8115-batch-priority-fees-at-end-of-block/27358)
- **Prior fork history:** none

## TL;DR
Stop crediting the EIP-1559 priority fee (tip) to the fee recipient after every transaction; instead sum all tips and credit the coinbase once, after the last transaction and before withdrawals. Removes a per-transaction write to the coinbase account, which unblocks parallel execution and kills "sender becomes solvent mid-block" edge cases.

## What it changes
- Consensus rule change in transaction processing: per-transaction priority-fee credits are removed; one batched credit of the block's total tips to `coinbase` happens after all transactions, before EIP-4895 withdrawals are processed.
- Sender balance deduction is unchanged (fees still deducted per transaction up front); only the credit side moves.
- The fee recipient's balance during block execution no longer includes the block's tips — they can only be spent from the next block on.
- Simplifies ETH-transfer logging: one fee transfer per block instead of hundreds of micropayments (relevant to transfer-log proposals like EIP-7708).
- State-root / receipt consequences: the coinbase account gets one write per block; in-block `eth_getBalance(coinbase)` semantics at intermediate positions change.

## Motivation
1. **Parallelization:** every transaction currently writes the fee-recipient balance, forcing a serialization point across otherwise-independent transactions.
2. **Mempool/validity complexity:** a sender can be insolvent at block start and become solvent mid-block via incoming tips — batching removes this ordering dependency.
3. **Accounting noise:** hundreds of tiny coinbase credits per block.
4. **Accurate logs:** ETH-transfer logs can record priority fees correctly as a single credit.

## Dependencies & related EIPs
- **Depends on:** EIP-1559 (fee market), EIP-4895 (withdrawals — defines the ordering point the batch credit is inserted before). Both long shipped.
- **Synergizes with:** EIP-8116 (same authors, same theme of making per-transaction outputs position-independent), EIP-7928 (block access lists — fewer per-tx coinbase writes shrink the BAL), EIP-7807 (SSZ execution blocks, same authors' package of block-format cleanups), and parallel-execution work generally.
- **Interacts with:** EIP-7708-style ETH transfer logs (single fee log per block); block-building / MEV infrastructure assumptions.

## Impact on ethrex / client teams
- Touches: post-transaction fee settlement — in ethrex this is `pay_coinbase` in `crates/vm/levm/src/hooks/default_hook.rs:504`, called from `finalize_execution` per transaction; it would move to block-level post-processing (payload/block finalization in `crates/blockchain/payload.rs` / VM backend) with an accumulated tip counter.
- Also affects: BAL/witness recording (coinbase touched once per block), block building, and any RPC/simulation paths (`eth_call`, traces) that expose intermediate coinbase balances.
- Tx pool can drop solvency-ordering edge cases (a mild simplification).
- Effort: **small** — a well-localized execution-semantics change; main cost is test updates (every fixture asserting per-tx coinbase balances) and MEV/builder-compat analysis.

## Open questions & controversies
- Fee recipient liquidity: builders/validators can no longer spend tips within the same block (relevant to MEV flows that fund later transactions from tip income).
- Block builder infrastructure needs updating; timing assumptions in MEV-Boost-style pipelines may shift.
- Changes observable intermediate state (traces, `debug` APIs) — tooling must adapt.
- Otherwise a low-controversy, well-understood change; spec is short and stable.

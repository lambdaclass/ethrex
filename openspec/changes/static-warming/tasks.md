## 1. Create static_warming module

- [x] 1.1 Create `crates/vm/backends/levm/static_warming.rs` module file
- [x] 1.2 Add `mod static_warming;` to `crates/vm/backends/levm/mod.rs`
- [x] 1.3 Add `pub mod static_warming;` to `crates/vm/backends/levm/mod.rs`

## 2. Implement call target extraction

- [x] 2.1 Implement `extract_call_targets(txs: &[Transaction]) -> Vec<Address>` to extract all `tx.to()` addresses
- [x] 2.2 Filter out addresses where `tx.to()` is `None` (these are CREATE transactions)

## 3. Implement CREATE address prediction

- [x] 3.1 Implement `predict_create_addresses(txs: &[Transaction], sender_nonces: &HashMap<Address, u64>) -> Vec<Address>`
- [x] 3.2 For CREATE: compute address from sender + nonce using rlp encoding
- [ ] 3.3 For CREATE2: extract salt from tx data (bytes after 0x until initcode), compute address from sender + salt + initcode_hash (not implemented - requires initcode at prediction time)

## 4. Implement bytecode static analysis

- [x] 4.1 Implement `extract_static_storage_keys(code: &[u8]) -> Vec<H256>` to scan bytecode for PUSH1/PUSH2 + SLOAD patterns
- [x] 4.2 Handle PUSH1 (0x60) with one-byte slot value
- [x] 4.3 Handle PUSH2 (0x61) with two-byte slot value
- [x] 4.4 Skip pattern if PUSH is followed by anything other than SLOAD (0x54)

## 5. Implement warm_block_static orchestration

- [x] 5.1 Implement `warm_block_static(block: &Block, store: Arc<dyn Database>) -> Result<(), EvmError>`
- [x] 5.2 Extract call targets from transactions
- [x] 5.3 Predict CREATE addresses (use approximate nonce based on tx position)
- [x] 5.4 Batch prefetch accounts via `store.prefetch_accounts()`
- [x] 5.5 For each contract with code, run bytecode analysis
- [x] 5.6 Batch prefetch storage slots via `store.prefetch_storage()`

## 6. Add benchmarking

- [ ] 6.1 Add benchmark comparing `warm_block` (old) vs `warm_block_static` (new) execution time
- [ ] 6.2 Run on same block to measure CPU time reduction

## 7. Integrate with execute_block_pipeline

- [x] 7.1 Modify `execute_block_pipeline` in `crates/vm/backends/levm/mod.rs`
- [x] 7.2 Replace call to `warm_block()` with `warm_block_static()` for non-BAL path
- [x] 7.3 Keep `warm_block` as fallback if static warming fails

## 8. Testing and verification

- [ ] 8.1 Run block execution tests to verify results unchanged
- [ ] 8.2 Run on mainnet blocks (pre-Amsterdam) to verify warming works
- [ ] 8.3 Monitor performance metrics in production

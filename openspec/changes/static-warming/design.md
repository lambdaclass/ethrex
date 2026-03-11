## Context

Current block execution in ethrex uses a 3-thread pipeline: warming, execution, and merkleization. For pre-Amsterdam blocks (without Block Access List), the warming phase runs `warm_block()` which re-executes every transaction to discover which accounts and storage slots get accessed. This is wasteful — full EVM execution just to collect access patterns.

The non-BAL path is used for:
- Pre-Amsterdam blocks (mainnet before timestamp 1753584000)
- P2P sync (when receiving blocks without pre-computed BAL)

## Goals / Non-Goals

**Goals:**
- Eliminate speculative EVM execution in the warming phase
- Use static analysis to extract account/storage access patterns from transactions
- Maintain identical block execution results
- Reduce CPU time in the warming phase by ~50%+

**Non-Goals:**
- This does NOT apply to Amsterdam+ blocks — they already use BAL-based warming (`warm_block_from_bal`)
- Does NOT change the actual block execution path
- Does NOT modify the merkleization phase
- Does NOT add new instrumentation — existing metrics remain unchanged

## Decisions

### 1. Static Extraction vs Hybrid Approach

**Decision:** Pure static extraction with bytecode analysis

**Rationale:** The alternative (hybrid — still re-execute but less) leaves the complexity of EVM execution in the warming path. Pure static is simpler and faster.

**Alternatives considered:**
- Hybrid (re-execute + static): More accurate but still has EVM overhead
- No bytecode analysis: Only warm accounts, skip storage — too aggressive, would cause cold reads

### 2. Bytecode Analysis: Simple Pattern Matching

**Decision:** Scan for `PUSH1/PUSH2 + SLOAD` patterns only

**Rationale:** Complex bytecode analysis (dataflow analysis, symbolic execution) is error-prone and expensive. Simple pattern matching catches the majority of static storage accesses (ERC20 balanceOf, mappings) with minimal code.

**Alternatives considered:**
- Full dataflow analysis: Overkill for warming, would add significant complexity
- Only warm accounts, no storage: Would cause cold reads on first SLOAD

### 3. CREATE Address Prediction

**Decision:** Predict CREATE2 addresses; accept that CREATE addresses may be slightly off

**Rationale:** CREATE2 is deterministic from sender + salt + initcode hash. CREATE depends on sender's nonce at execution time, which we can approximate from tx position in block.

**Alternatives considered:**
- Don't predict CREATE at all: Would cause cold access for new contracts — acceptable
- Full nonce tracking: Complex, requires maintaining nonce state across txs

### 4. Module Location

**Decision:** New `static_warming.rs` in `crates/vm/backends/levm/`

**Rationale:** Keeps the warming logic isolated from the main execution path. Easy to benchmark and compare against `warm_block`.

## Risks / Trade-offs

### Risk: Bytecode Analysis Miss Rate

**Problem:** Some contracts use dynamic storage keys (e.g., `slot = keccak256(msg.sender || id)`)

**Mitigation:** Accept ~20% miss rate. Storage values will still load, just not pre-warmed. Gas cost increases (cold read penalty ~2100 gas), but warming overhead eliminated.

### Risk: CREATE Address Accuracy

**Problem:** CREATE address depends on sender's nonce at time of execution, which may differ from our prediction if there are other transactions from the same sender in the block

**Mitigation:** If predicted address doesn't exist in database, prefetch is a no-op. No harm done.

### Risk: Code Not Available

**Problem:** Called contract's code might not be in database during warming (e.g., newly deployed contract)

**Mitigation:** Handle missing code gracefully — just skip bytecode analysis for that contract. Account is still prefetched.

### Trade-off: Accuracy vs Speed

The static approach is faster but less accurate than speculative execution. However, the goal is to warm state, not to validate execution. The actual execution phase still runs and produces correct results.

## Migration Plan

1. **Implement** `static_warming.rs` module
2. **Add** benchmarks comparing `warm_block` vs `warm_block_static` on same block
3. **Switch** `execute_block_pipeline` to use new implementation
4. **Deploy** and monitor — verify block execution times improve
5. **Remove** old `warm_block` after confidence period

No migration needed — this is a transparent optimization. Blocks produce identical results.

## Open Questions

1. **Q: Should we also warm the EIP-2929 accessed_storage_slots sets?**
   - Currently warming only loads values, not the warm/cold tracking
   - Could initialize `accessed_storage_slots` from bytecode analysis too
   - Defer to future work if profiling shows benefit

2. **Q: How to verify correctness?**
   - Run blocks with both old and new warming, compare results
   - No differences expected — can be part of CI

3. **Q: What about revert paths?**
   - Static warming doesn't execute, so no revert logic needed
   - Simpler than the old approach

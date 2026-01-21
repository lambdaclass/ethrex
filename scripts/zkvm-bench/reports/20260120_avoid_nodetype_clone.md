# Optimization Report: Avoid NodeType Cloning in EncodedTrie

## Change
Modified `authenticate()` and `hash()` in `crates/common/trie/encodedtrie.rs` to avoid cloning `NodeType` during recursive traversal.
Instead of `match &trie.nodes[index].node_type.clone()`, we now inspect `node_type` by reference.
For recursion, we extract `child_index` or `children_indices` upfront (copying `usize` or array of `usize` is cheap) to avoid borrowing conflicts, then perform recursion, then inspect `node_type` again by reference for the post-recursion logic.

## Files Modified
- `crates/common/trie/encodedtrie.rs`

## Results

| Metric | Baseline | After | Change |
|--------|----------|-------|--------|
| Total Steps | 501,984,983 | 498,004,423 | -3,980,560 (-0.79%) |
| EncodedTrie::authenticate::recursive | 18.47B (cost) | 18.08B (cost) | -2.09% |
| EncodedTrie::hash::recursive | 20.90B (cost) | 20.58B (cost) | -1.58% |
| memcpy calls | 805,839 | 789,487 | -16,352 |

## Analysis
The `authenticate` and `hash` functions were cloning the `NodeType` enum at every step of the recursion. `NodeType` can be large (especially `Branch` variant which has an array of 16 options, or `Leaf` which has `Vec<u8>`).
By avoiding this clone:
1. We reduced memory allocation and copying (memcpy).
2. We reduced the instruction count in the hot recursive paths.

The profile shows a clear reduction in `memcpy` calls and costs, and a direct improvement in the `authenticate` and `hash` functions.

## Next Steps
- Update baseline to this new state.
- Investigate other sources of `memcpy` (still ~19% of cost).
- Look into `TinyKeccak::update` optimization (23% of cost).

# Verification Gates

7-item mandatory checklist before any proof is considered complete.
Derived from lambda_compiler_kit battle-tested protocol.

## The Gate

```
[ ] 1. grep sorry — no sorry remaining in file
[ ] 2. lake build — compiles without errors
[ ] 3. grep wfail — no expected-failure markers left
[ ] 4. check_axioms_inline.sh — only standard axioms (propext, quot.sound, Classical.choice)
[ ] 5. proof explanation — brief comment explaining non-obvious steps
[ ] 6. examples preserved — existing #guard/example still pass
[ ] 7. imports minimal — no unnecessary imports added
```

## Rationale

- **Gates 1-2**: Basic correctness. A proof with sorry or compile errors is not a proof.
- **Gate 3**: wfail markers are temporary development aids. They must not ship.
- **Gate 4**: Custom axioms defeat the purpose of formal verification. The only acceptable axioms are Lean's three foundational ones.
- **Gate 5**: Proofs are read more than written. A comment explaining "why this approach" saves future debugging time.
- **Gate 6**: Regression prevention. New proofs must not break existing verified properties.
- **Gate 7**: Import hygiene prevents slow builds and dependency creep.

## Architecture Before Tactics

**Key insight from lambda_compiler_kit**: When a proof is stuck, the problem is almost always in the STATEMENT, not in the tactics.

Before spending time on tactics:
1. Is the statement actually true? (test with examples)
2. Are the types right? (check with #check)
3. Is the abstraction level appropriate? (too general = unprovable)
4. Does the induction principle match the recursion? (structural vs well-founded)

**The CPS insight**: Pure CPS (Continuation-Passing Style) transforms make termination proofs trivial because the recursive structure becomes explicit. When stuck on termination, consider whether a CPS transform resolves the issue.

## Firewall Pattern

For files imported by 3+ other files (high fan-out):

1. Create `theorem foo_aux` with a flexible signature
2. Prove `foo_aux` completely (no sorry)
3. Only then replace the original `foo` with `foo_aux`'s content
4. If `foo_aux` fails: the original is untouched, no cascade of breakage

This prevents the "edit foundational file → 15 files break → panic revert" cycle.

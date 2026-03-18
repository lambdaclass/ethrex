# Compiler-Guided Proof Repair

**Core insight:** Use Lean's compiler feedback to drive iterative repair with small, budgeted LLM calls instead of blind best-of-N sampling.

**Inspired by:** APOLLO (https://arxiv.org/abs/2505.05758)

## Philosophy

**Traditional (blind sampling):**
```
Generate 100 proof attempts -> Test all -> Pick best
Problem: Most attempts fail identically. No learning.
```

**Compiler-guided:**
```
Generate attempt -> Lean error -> Route to specific fix -> Retry (max 24 attempts)
Win: Error-driven action selection. Different fix per error type.
```

## Repair Loop

```
1. Compile file
2. If success -> DONE
3. Parse error -> classify (errorStrategies.yaml)
4. Try solver cascade (handles 40-60% of cases)
5. If solver works -> apply diff -> goto 1
6. If not -> route to error-specific action chain
7. If action works -> apply -> goto 1
8. If 3 identical errors -> escalate (Haiku -> Opus)
9. If still failing -> reformulate statement
10. Max 24 attempts total
```

## Solver Cascade

Try in order (each takes 1-8 seconds):
1. `rfl` — definitional equality
2. `simp` — simplifier
3. `ring` — ring normalization
4. `linarith` — linear arithmetic
5. `nlinarith` — nonlinear arithmetic
6. `omega` — integer arithmetic
7. `exact?` — proof search
8. `apply?` — proof search
9. `aesop` — general automation

## Error Classification

See `config/errorStrategies.yaml` for the full routing table.

Key patterns:
- `type mismatch` -> simp/rfl/ring, then refine skeleton
- `unsolved goals` -> simp?/apply?/exact?/aesop, then intro/cases
- `unknown identifier` -> search Mathlib, open namespace
- `failed to synthesize instance` -> haveI/letI, open scoped
- `deterministic timeout` -> simp only with explicit lemmas

## Two-Stage Model Escalation

- **Stage 1**: Fast model (Haiku) with K=1 sampling
- **Stage 2**: Strong model (Opus) after 3 Stage 1 failures on same error
- **Bail**: After 3 Stage 2 failures on identical error

## Key Metrics

- **Attempts**: Track total and per-error-type
- **Sorry count**: Track reduction over time (Ax-Prover pattern)
- **Repeat ratio**: If >50% of attempts hit same error, escalate or reformulate

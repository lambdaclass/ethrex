# ethrex Review Guide

## What to Review (DO comment on these)

### Correctness & Logic
- **Off-by-one errors** in loops, ranges, slice indexing
- **Integer overflow/underflow** — especially in gas calculations, balance operations, RLP length prefixes
- **Missing error propagation** — bare `.unwrap()` in non-test code, swallowed errors, `let _ = ...` on Results that matter
- **Incorrect early returns** — returning Ok when an error condition was detected
- **Race conditions** — shared mutable state without proper synchronization, TOCTOU bugs
- **Deadlocks** — lock ordering violations, holding locks across await points

### Ethereum-Specific
- **RLP encoding/decoding symmetry** — if you encode then decode, do you get the same value? Check `RLPEncode` and `RLPDecode` impls match
- **Gas accounting** — gas charged matches spec, gas refunds handled correctly, gas limit checks present
- **Fork boundaries** — code that should only run after a specific fork (Shanghai, Cancun, Prague, Osaka) has proper fork checks
- **Merkle Patricia Trie operations** — correct key encoding (secure vs raw), proper handling of empty/deleted nodes
- **EIP compliance** — implementation matches the referenced EIP spec, especially edge cases mentioned in the EIP

### Storage & State
- **Store error handling** — Store operations can fail (disk full, corruption); check errors are propagated not ignored
- **State consistency** — operations that modify multiple storage keys should be atomic or handle partial failure
- **Trie cache invalidation** — changes to state should properly invalidate cached trie nodes
- **STORE_SCHEMA_VERSION** — if Store schema changes, the version constant must be bumped

### Concurrency
- **Lock scope** — locks held for minimum necessary duration; no I/O, allocations, or complex computation while holding a lock
- **Lock ordering** — consistent acquisition order across the codebase to prevent deadlocks; document expected order if non-obvious
- **Async/await** — no blocking calls (std::sync::Mutex::lock, std::fs, sleep) inside async contexts; use tokio equivalents
- **Holding locks across await** — a `MutexGuard` held across an `.await` point can deadlock or starve other tasks
- **Channel usage** — unbounded channels in hot paths can cause OOM; prefer bounded channels with backpressure
- **Shutdown signaling** — tasks and threads must have a clean shutdown path; dangling spawned tasks leak resources
- **Shared mutable state** — Arc<Mutex<T>> vs message passing; prefer message passing when state is accessed from many places
- **Atomics misuse** — wrong `Ordering` (e.g., `Relaxed` when `Acquire`/`Release` is needed), non-atomic read-modify-write patterns
- **Spawned tasks without join** — fire-and-forget `tokio::spawn` without storing the JoinHandle makes errors silent and shutdown unclean
- **Concurrent map access** — HashMap/BTreeMap accessed from multiple threads without synchronization (even via unsafe)

### Performance
- **O(n^2) or worse** — unnecessary nested loops, repeated linear searches, `.contains()` in a loop over a Vec that should be a HashSet
- **Repeated allocations in loops** — String/Vec/Box created inside tight loops that could be pre-allocated or reused
- **Cloning large structures** — cloning Bytes, Vec<u8>, Trie nodes, Blocks when a reference or Cow would suffice
- **Unnecessary collect** — `.collect::<Vec<_>>()` followed by `.iter()` when the iterator could be chained directly
- **String formatting in hot paths** — `format!()` or `.to_string()` in per-transaction or per-block loops; prefer write! to a reused buffer
- **Redundant hashing/serialization** — computing the same hash or RLP encoding multiple times when it could be cached or passed through
- **Large stack allocations** — big arrays or structs on the stack in recursive functions (trie traversal); prefer Box or Vec
- **Missing short-circuit** — doing expensive work before a cheap check that could bail early
- **Unbounded caches** — caches that grow without eviction policy, effectively a memory leak under load
- **Database round-trips in loops** — calling Store methods inside a loop when a batch/bulk operation exists

### Security
- **Command injection** — unsanitized input used in shell commands or system calls
- **Unchecked arithmetic** — in consensus-critical code, use `checked_add`/`checked_mul`/`saturating_*`; bare `+`, `-`, `*` on untrusted values
- **DoS vectors** — unbounded allocations from untrusted input, missing size limits on network messages, RLP decoding without length caps
- **Untrusted RLP/SSZ input** — decoding peer-supplied data without validating lengths, nesting depth, or field ranges
- **Timing side channels** — non-constant-time comparison of secrets (private keys, auth tokens); use `subtle::ConstantTimeEq`
- **Unsafe blocks** — every `unsafe` must have a `// SAFETY:` comment justifying why invariants hold; flag any that don't
- **Panic in consensus paths** — `.unwrap()`, `panic!()`, array index without bounds check in block validation/execution code; these crash the node
- **Unbounded recursion** — recursive trie traversal or RLP decoding on untrusted input without depth limits
- **Resource exhaustion** — accepting unbounded connections, unbounded pending transactions, or unbounded RPC request sizes
- **Private key / secret handling** — secrets should be zeroized after use (`zeroize` crate), not logged, not included in error messages

### Style, Naming & Formatting (prefix with "nit:")
These are lower priority than correctness — always prefix with `nit:`.
- **Naming clarity** — variable/function names that are misleading, overly abbreviated, or don't convey intent (e.g., `x` for a block number, `do_thing` for a complex operation)
- **Inconsistent naming** — not following conventions used elsewhere in the codebase (e.g., `get_` vs `fetch_` when the crate consistently uses one)
- **Visibility** — `pub` on things that should be `pub(crate)` or private, leaking internal details
- **Import hygiene** — wildcard imports (`use module::*`) in non-test code, duplicated imports, unused imports
- **Formatting issues** — that rustfmt wouldn't catch: inconsistent spacing in comments, badly formatted multiline expressions, poor alignment in match arms
- **Idiomatic Rust** — `if let` vs `match` for single-arm, `.map()` vs `for` loops, iterator chains vs manual loops, `.clone()` when a borrow works
- **Dead code** — unused functions, unreachable branches, commented-out code left in

### Documentation (prefix with "nit:")
Also lower priority — prefix with `nit:`.
- **Missing doc comments** on public API items (pub fn, pub struct, pub enum) that aren't self-explanatory
- **Incorrect or outdated doc comments** — docs that describe behavior that doesn't match the implementation
- **Missing context** — complex algorithms or non-obvious logic without any explanation
- **Missing safety comments** on `unsafe` blocks

## What NOT to Review (DO NOT comment on these)

- TODOs or FIXMEs already present in code (not introduced by this PR)
- Commit message format or PR description quality
- Test structure or test naming conventions
- Import ordering (handled by tooling)

## Review Tone

- Be specific: cite the exact line and explain the concrete issue
- Suggest fixes when possible — don't just point out problems
- Use "nit:" prefix for style, naming, formatting, and docs comments — this signals lower priority
- If unsure about something, phrase it as a question: "Does this handle the case where X is empty?"
- Acknowledge good patterns when you see them (briefly)
- Don't be condescending or overly verbose

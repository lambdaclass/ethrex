# EIP-8058: Contract Bytecode Deduplication Discount
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** none recorded (not re-proposed for Hegota in forkcast data as of this writing)
- **Authors:** Carlos Perez, Wei Han Ng, Guillaume Ballet
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8058) · [discussion](https://ethereum-magicians.org/t/eip-8058-contract-bytecode-deduplication-discount/25933)
- **Prior fork history:** Declined from Glamsterdam (acde/227, 2026-01-05)

## TL;DR
If a transaction's EIP-2930 access list contains an address whose deployed bytecode hash equals the bytecode a CREATE/CREATE2 is deploying, the deployer skips paying `GAS_CODE_DEPOSIT * len(code)` — the new account just links to the already-stored code hash. Clients already dedupe bytecode internally, so the discount aligns gas with actual storage use. It becomes much more valuable under EIP-8037, which raises code deposit cost from 200 to 1,900 gas/byte (a 24 kB contract: ~46.6M gas of deposit alone).

## What it changes
- No new transaction fields; reuses EIP-2930 access lists unchanged.
- At transaction start, build a "CodeHash Access-Set" `W = { codeHash(a) | a in access list, a exists, a has code }` from pre-execution state. Reading those code hashes is already covered by EIP-2929/2930 access costs; no extra charge for the check.
- On successful CREATE/CREATE2 returning bytecode `B` of length `L`: compute `H = keccak256(B)`. If `H ∈ W`, link the new account's codeHash to `H` without charging code-deposit gas. Otherwise charge `GAS_CODE_DEPOSIT * L` and persist `B` as today.
- Same-block chaining works: a later tx can reference an address deployed earlier in the same block, since `W` is built from current (post-prior-tx) state.
- Simultaneous new deployments of identical bytecode with no access-list reference both pay full price — dedup is opt-in via the access list, deliberately.

## Motivation
Clients store bytecode once per code hash regardless of how many accounts reference it, yet every duplicate deployment pays the full deposit. A naive "check if code exists in my database" discount breaks consensus: full-sync and snap-sync nodes hold different code sets (~27,869 orphaned bytecodes existed in full-sync DBs at Cancun). Keying the discount to the signed access list makes the check deterministic and identical on all nodes, with no codeHash→accounts reverse index and no code-root consensus requirement.

## Dependencies & related EIPs
- **Depends on:** EIP-2930 (access lists, Berlin) — the mechanism is built entirely on it; EIP-2929 warm/cold costing covers the reads.
- **Synergizes with:** EIP-8037 (Glamsterdam state-gas repricing) — the economic case depends on the raised `GAS_CODE_DEPOSIT` (1,900/byte); without it the discount is marginal. EIP-7981 (access-list byte floor) prices the extra access-list bytes used to claim the discount.
- **Alternatives:** the spec explicitly rejects DB-lookup dedup (consensus-unsafe) and reverse-index approaches (complexity); no competing Hegota candidate.
- **Cluster context:** the bytecode-dedup entry of the gas repricing cluster.

## Impact on ethrex / client teams
- Medium. Touches tx-start preparation (build `W` from the access list while warming, in `crates/vm/levm/src/vm.rs` `prepare_vm`/initial-access logic around `crates/vm/levm/src/vm.rs:3432`) and the CREATE/CREATE2 completion path (`crates/vm/levm/src/opcode_handlers/system.rs`) where code-deposit gas is charged.
- No trie/schema changes, no new tx type, no networking/Engine API changes. The code-dedup storage itself already exists client-side.
- RPC/tooling: `eth_estimateGas` should account for the discount when access lists are present (possibly dual estimates); the spec suggests (optional) a code→address reverse index to help users find a reference address.
- Edge cases to test: same-block deployment ordering, self-destructed/recreated references, EIP-8037 state-gas interaction.

## Open questions & controversies
- Declined from Glamsterdam (acde/227) — was deprioritized rather than rejected on merits; champion (CPerezz) would need to re-propose for Hegota, and forkcast shows no Hegota entry yet.
- Benefit is opt-in and ordering-sensitive: wallets/builders must add the right address to the access list, and the discount can be lost to same-block ordering games (noted in spec as formalized but complex).
- Gas-estimation UX is awkward: estimators must know whether the referenced code hash will match, which depends on execution outcome.
- Adds a new consensus-visible dependency between the access list and execution gas costs that some reviewers see as scope creep for access lists.

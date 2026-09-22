# EIP-4758: Deactivate SELFDESTRUCT
- **Layer:** EL
- **EIP status:** Stagnant
- **Hegota status:** Proposed (CFI) — presented at acde/237, 2026-05-21 (champion: Peter Miller)
- **Authors:** Guillaume Ballet, Vitalik Buterin, Dankrad Feist
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-4758) · [discussion](https://ethereum-magicians.org/t/eip-4758-deactivate-selfdestruct/8710)
- **Prior fork history:** none (its predecessor EIP-6780 shipped in Cancun as a partial restriction)

## TL;DR
This EIP renames SELFDESTRUCT to SENDALL and strips it down to a single behavior: transfer the account's entire ETH balance to the target. It no longer deletes code or storage or touches the nonce. This completes the SELFDESTRUCT deprecation arc that EIP-6780 started in Cancun, and is required for the Verkle/stateless roadmap where an account's storage is spread across many unrelated tree keys and cannot be wiped wholesale.

## What it changes
- SELFDESTRUCT (0xff) becomes SENDALL: only moves all ETH in the account to the beneficiary target.
- No deletion of code or storage, no nonce alteration — regardless of whether the contract was created in the same transaction (closing the EIP-6780 same-tx exception).
- All SELFDESTRUCT-related gas refunds are removed.
- Backwards-breaking for: CREATE2 redeploy at the same address after selfdestruct (used by some "metamorphic" upgradeability patterns), selfdestruct-based burn of non-ETH token balances, and contracts relying on destruction for access control.

## Motivation
SELFDESTRUCT is the only opcode that erases an account's code and storage wholesale. Under Verkle trees each account's data lives at many unconnected keys, making deletion infeasible; it is also a blocker for statelessness and snapshot-style sync. The useful behavior (recovering funds) is preserved. Cancun's EIP-6780 already restricted deletion to same-transaction-created contracts; this finishes the job and removes the remaining special-case machinery.

## Dependencies & related EIPs
- Builds on EIP-6780 (Cancun), which was the intermediate restriction step.
- Motivated by the Verkle/statelessness roadmap (EIP-6800 lineage; not itself on the Hegota list).
- Related Hegota candidates: EIP-3298 (removing gas refunds entirely — complementary), EIP-2488 (CALLCODE deprecation — same "dead opcode cleanup" family).
- The staged deprecation (EIP-6049 → EIP-6780 → EIP-4758) is cited by EIP-8365/8367 as the precedent for retiring BLS withdrawal credentials.

## Impact on ethrex / client teams
- Small-to-medium. The SELFDESTRUCT handler lives in `crates/vm/levm/src/opcode_handlers/system.rs`, with account-touch/delete logic in `crates/vm/levm/src/vm.rs` and `crates/vm/levm/src/account.rs`; ethrex also has BAL (block access list) handling for selfdestruct reads (`crates/common/types/block_access_list.rs`, tests in `test/tests/blockchain/bal_*`) that would need fork-gating.
- Gas cost table (`gas_cost.rs`) loses the refund paths.
- Impact on state/trie: none directly, but this is a prerequisite for future Verkle work.
- RPC/Engine API untouched. Trace output (`crates/common/tracing.rs`) may need updating to reflect SENDALL semantics.

## Open questions & controversies
- Breaks metamorphic-contract upgradeability and some deployed patterns; ecosystem breakage analysis has historically been the main pushback (the reason only EIP-6780 shipped in Cancun instead of this).
- Spec is short and Stagnant; no test cases or reference implementation. The exact interaction with BAL (EIP-7928, Glamsterdam) bookkeeping for a now non-deleting SELFDESTRUCT needs pinning down.
- Some argue the marginal benefit over EIP-6780 is small until Verkle actually lands, so timing it with state-tree work is contested.

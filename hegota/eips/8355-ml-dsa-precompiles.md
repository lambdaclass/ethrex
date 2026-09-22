# EIP-8355: Precompiles for ML-DSA Verification
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/243, 2026-08-13 (champion: Danno Ferrin)
- **Authors:** Danno Ferrin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8355) · [discussion](https://ethereum-magicians.org/t/eip-8355-precompiles-for-ml-dsa-verification/29211)
- **Prior fork history:** none

## TL;DR
Adds three precompiles that verify ML-DSA (FIPS 204, the NIST-standardized lattice-based post-quantum signature scheme) at all three parameter sets — ML-DSA-44/65/87, NIST security levels II/III/V. Input is a bare `pubkey ++ signature ++ message` concatenation; output is always a 32-byte word (1 = valid, 0 = invalid). A post-quantum signature verifier contracts can call.

## What it changes
- New precompiles at `0x12` (ML-DSA-44), `0x13` (ML-DSA-65), `0x14` (ML-DSA-87) — previously unassigned addresses.
- Input format: `pubkey (PK_LEN) ++ signature (SIG_LEN) ++ message (rest)`, no length prefixes, no header. Compressed FIPS 204 encodings (`pk = (ρ, t1)`); message is variable-length, empty message allowed. Fixed sizes: 44 → 1312/2420 bytes; 65 → 1952/3309; 87 → 2592/4627.
- Semantics: full `ML-DSA.Verify` per FIPS 204 with empty context string; malformed encodings (bad coefficients, non-canonical hints, `‖z‖` bound failure) return 0 rather than revert. **Never reverts** except out-of-gas.
- Output: always exactly 32 bytes — deliberately different from ECRECOVER / P256VERIFY's empty-on-failure, so callers can distinguish "invalid signature" from "precompile not present on this chain" (returndata length ≠ 32).
- Gas: `BASE + 6 * ceil(max(0, L − PK_LEN − SIG_LEN)/32)`, charged fully **before** any verification. BASE = 6500 / 9000 / 13500 for 44/65/87. The 6 gas/word matches the `KECCAK256` rate (SHAKE256 and Keccak share the same permutation).
- Test vectors: Wycheproof `mldsa_44_verify_test.json` cases included.

## Motivation
Contracts currently have no way to verify a post-quantum signature. ML-DSA is the first finalized NIST lattice standard, and the HSM/cloud-KMS/certificate ecosystem has converged on FIPS 204 encodings — a precompile consuming those encodings unmodified lets accounts be backed by keys that never leave hardware. Differs from the competing EIP-8051 design: covers all three NIST levels (8051 has only ML-DSA-44), takes variable-length messages instead of forcing a 32-byte pre-hash (pre-hashing is actually a distinct construction, HashML-DSA, per FIPS 204 §5.4), and takes compressed public keys (8051's pre-expanded keys are 15× larger and paid per-transaction in witness bandwidth).

## Dependencies & related EIPs
- **Alternative to / competes with:** EIP-8051 (ML-DSA-44-only, pre-expanded key, fixed 32-byte message). 8051 is not on the Hegota candidate list, so 8355 is the live PQ-verifier candidate.
- **Synergizes with:** EIP-8141 (frame transactions / native AA) — an ML-DSA witness can arrive as an `ARBITRARY` signature entry holding `pubkey ++ signature`, which a `VERIFY` frame copies into memory with one `SIGPARAM`; appending the message yields precompile input with zero repacking.
- **Depends on:** nothing consensus-level; standalone precompile addition. Gas figures still need benchmarking.

## Impact on ethrex / client teams
- Touches: precompile table and dispatcher (`crates/vm/levm/src/precompiles.rs`), gas constants, and a new crypto backend (an ML-DSA crate — the spec notes pure-Rust, no-std, constant-time, and formally verified options exist) in `crates/common/crypto`.
- Also needs wiring into the guest-program crypto provider (`crates/guest-program/stateless-validator/src/crypto/`) so ZK proving can verify these calls.
- No changes to tx pool, networking, Engine API, or RPC.
- Effort: **small-to-medium** — well-specified, bounded input, existing audited libraries; main work is integration, gas benchmarking, and ZK-prover support.

## Open questions & controversies
- Very new (July 2026 draft); gas schedule explicitly "requires benchmarking against a reference implementation before final."
- Design deliberately breaks caller expectations from `ecrecover`/P256VERIFY: success of the CALL says nothing about validity; empty returndata means "precompile absent", not "bad signature". Ported contracts that treat empty returndata as failure will misbehave.
- No domain separation (empty FIPS 204 context) — applications must bind chain/contract/intent into the message themselves.
- Broader controversy: whether to ship PQ verification now vs. waiting for a full PQ account-abstraction story; and whether ML-DSA is the right scheme vs. hash-based alternatives.

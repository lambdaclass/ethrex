# EIP-8131: Unified Transaction Content Floor
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/238, 2026-06-04)
- **Authors:** Toni Wahrstätter
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8131) · [discussion](https://ethereum-magicians.org/t/eip-9999-add-auth-data-to-eip-7623-floor/12345) *(note: the discussion URL in both the spec and forkcast metadata looks like a placeholder — topic 12345, "eip-9999")*
- **Prior fork history:** none

## TL;DR
Extends the EIP-7623-style gas floor to *every* user-controlled byte in a transaction — calldata, access-list entries, EIP-7702 authorization tuples, and EIP-4844 blob versioned hashes — at one flat rate of 64 gas per byte. One formula replaces EIP-7976 (calldata) and EIP-7981 (access lists) and closes the gap where auth tuples and blob hashes pay nothing at the floor. Worst-case attacker-controlled content per block becomes provably ≤ `block_gas_limit / 64` ≈ 0.89 MB at a 60 M gas limit.

## What it changes
- **Floor formula** (constants: `FLOOR_GAS_PER_BYTE = 64`, `AUTH_TUPLE_BYTES = 108`, `BLOB_VERSIONED_HASH_BYTES = 32`, access-list address 20 B / key 32 B):
  ```
  tx_floor = 21000 + 64 * ( len(data)
                          + 20 * num_access_list_addresses
                          + 32 * num_access_list_storage_keys
                          + 108 * num_authorizations
                          + 32 * num_blob_versioned_hashes )
  ```
- **Charging:** `require tx.gas >= max(intrinsic, tx_floor)` and `gasUsed = max(execution_gas_used, tx_floor)` — the EIP-7623 `max(intrinsic, floor)` shape is preserved; standard intrinsic gas (EIP-2028: 4/16 gas per zero/non-zero calldata byte) is unchanged.
- Floor is computed from **content bytes only** (not full RLP envelope/signature, which `TX_BASE = 21000` covers at ~130 B worst case) so all tx types with identical content pay identical floor.
- `AUTH_TUPLE_BYTES = 108` is a worst-case constant (max RLP of an EIP-7702 tuple), so the floor is computable from counts alone; typical auths over-pay ~1,024 gas.
- Measured impact (May 2026 mainnet sample, 441k txs): aggregate bind rate rises 1.49% → 3.28%, total network gas +3.56%; type-4 (SetCode) bind rate jumps 0.04% → 20.6% because auth tuples were previously free at the floor.
- Requires wallets / `eth_estimateGas` to compute the new floor or submissions are rejected.

## Motivation
Every node must handle the *largest* block an attacker can build, not the average. EIP-7623 capped calldata-driven block bloat, 7976/7981 unified the rate at 64 gas/B for calldata and access lists — but EIP-7702 authorization tuples (up to 108 B each) and EIP-4844 blob versioned hashes (32 B each) were added with no floor term, leaving free bytes at the floor. One flat per-byte rule closes both gaps, auto-prices any future variable-length field, and yields a worst-case content bound provable from arithmetic alone (no compression assumptions).

## Dependencies & related EIPs
- **Requires / folds in:** EIP-2028 (intrinsic calldata cost), EIP-7623 (floor mechanism), EIP-7976 (64 gas/B calldata floor), EIP-7981 (access-list floor) — 8131 supersedes the 7976+7981 pair with a single rule — plus EIP-7702 (auth tuples) and EIP-4844 (blob hashes) as newly priced fields. 7976 and 7981 are Amsterdam-era; ethrex already implements them (see below).
- Synergizes with **EIP-7702**-heavy adoption (Pectra shipped) and blob scaling (**EIP-7594** PeerDAS in Fusaka): the more auth/blob traffic grows, the bigger the unpriced-floor gap this closes.
- Related family in the Hegota candidate list: **EIP-7976** and **EIP-7981** (as the proposals being subsumed); general block-size/DoS theme overlaps with **EIP-7825**-style tx gas caps (already shipped in Fusaka/Osaka).

## Impact on ethrex / client teams
- **Floor computation:** ethrex already has the EIP-7623/7976/7981 machinery — `crates/vm/levm/src/utils.rs:974` (standalone floor fn), `crates/vm/levm/src/gas_cost.rs:355` (floor cost per token), fork-gated in `crates/vm/levm/src/hooks/default_hook.rs:608` and `l2_hook.rs:324`. 8131 means extending that formula with the auth-tuple and blob-hash terms and (if 7976/7981 land as scheduled) replacing the two field-specific terms with the unified one.
- **Tx pool & block validation:** mempool admission checks in `crates/blockchain/mempool.rs` (floor reservation logic at `mempool.rs:2095`) and payload building (`crates/blockchain/payload.rs:1367`) must use the new floor.
- **RPC:** `eth_estimateGas` / `eth_call` must return floor-aware estimates (`crates/networking/rpc/eth/transaction.rs`).
- No state, networking-protocol, or Engine API changes. Hard fork (consensus-relevant gas rule).
- Effort: **small** — the plumbing for a floor already exists; this is one formula change plus tests.

## Open questions & controversies
- Discussion link is a placeholder in the spec itself — the EIP looks freshly re-numbered/re-written; maturity is low despite the polished numbers.
- Depends on 7976/7981 (both only Amsterdam-stage) — if those slip, 8131's "fold into one rule" story changes.
- Type-4 (SetCode) transactions take the biggest hit (bind rate 0.04% → 20.6%); expect pushback from account-abstraction/delegation heavy users who see +6,912 gas per authorization.
- Wallets/estimators that don't update will produce rejected submissions — ecosystem coordination cost.
- +3.6% total network gas is a real, visible repricing; governance appetite for stacking it on top of Amsterdam's floor changes is untested.

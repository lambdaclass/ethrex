# EIP-8077: eth/XX - announce transactions with nonce
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acde/243, 2026-08-13)
- **Authors:** Csaba Kiraly
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8077) · [discussion](https://ethereum-magicians.org/t/eip-8077-eth-xx-add-nonce-and-source-to-transactions-announcement/26505)
- **Prior fork history:** none

## TL;DR
Extends the devp2p `eth` protocol's `NewPooledTransactionHashes` announcement to also carry each transaction's sender address and nonce, alongside the existing hash/type/size. This lets receiving nodes make intelligent fetch scheduling decisions — avoid pulling transactions that leave nonce gaps, fill gaps deterministically, and filter stale announcements against chain state.

## What it changes
- `NewPooledTransactionHashes (0x08)` gains two parallel lists: `[txsource₁: B_20, …]` and `[txnonce₁: P, …]` appended to the current `(eth/69)` layout `[txtypes, [txsizes], [txhashes]]`.
- Requires a new `eth` wire protocol version (eth/XX); no consensus change, no hard fork. Old clients keep using the previous version.
- Sender side is trivial (announce extra metadata); receiver-side scheduling improvements are explicitly left to implementations, not mandated.
- Implementations MAY revisit the eager-push vs announce threshold since announcements grow (a B_20 address + variable-length nonce per entry).
- Requires **EIP-7642** (eth/69, history expiry) as baseline.

## Motivation
Today announcements carry only hash/type/size, so the receiver cannot tell whether fetching a transaction yields something includable: pulling transactions from multiple peers easily creates nonce gaps in the local mempool view, gap filling is trial-and-error, filtering already-mined transactions requires caching every on-chain tx hash, and selective fetching by sender (relevant for UX and L2s) is impossible. As block throughput rises, nodes increasingly cannot fetch everything and need smarter scheduling.

## Dependencies & related EIPs
- Requires **EIP-7642** (eth/69 baseline protocol version).
- Same author and family as **EIP-8094** (blob-aware mempool); the spec notes the two "can be simply combined" — 8094's Option 2 even uses this nonce announcement to detect blob-tx replacements.
- Related to **EIP-8070** (sparse blobpool) only via 8094.
- Discussed variant: identifying transactions by (source, nonce, RBF-version) instead of txhash, or adding fee data to announcements — explicitly *not* chosen in the current draft.

## Impact on ethrex / client teams
Networking-only. ethrex already structures protocol versions per directory (`crates/networking/p2p/rlpx/eth/eth68 … eth72`), so a new eth/XX fits the existing pattern: extend the `NewPooledTransactionHashes` codec (`crates/networking/p2p/rlpx/eth/transactions.rs`), announcement handling in `crates/networking/p2p/rlpx/connection/server.rs`, and the broadcaster (`tx_broadcaster.rs`). The real work is optional-but-valuable: nonce-aware fetch scheduling and gap filling in the mempool/tx-fetch logic (`crates/blockchain/mempool.rs`, `crates/networking/p2p/peer_handler.rs`). Effort: small-to-medium.

## Open questions & controversies
- Bandwidth overhead is significant (address + nonce per announcement on top of hash/type/size); spec overhead analysis is still "TODO".
- Announced source/nonce is unverifiable until the transaction is fetched — must be treated as untrusted; a mismatch is handled as a protocol violation by the announcer.
- Benefit is not protocol-guaranteed: it depends entirely on implementations building smarter schedulers.
- Design space still open per the Rationale: probabilistic detailed-vs-simple announcements, fee-in-announcement variants, sender/nonce-based tx identity.

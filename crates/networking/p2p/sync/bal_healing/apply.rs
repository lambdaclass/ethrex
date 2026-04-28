//! `apply_bal`: apply a single `BlockAccessList` diff to a state trie.
//!
//! # Account destruction encoding (Task 0.5 resolution)
//!
//! EIP-7928 does **not** define an explicit destruction marker on `AccountChanges`.
//! The struct carries `balance_changes`, `nonce_changes`, `code_changes`,
//! `storage_changes`, and `storage_reads` — no `destroyed` field exists.
//!
//! Rule adopted for BAL replay (implicit-empty):
//!   An account is considered destroyed after applying all changes if and only if
//!   `balance == 0 AND nonce == 0 AND code_hash == EMPTY_KECCAK_HASH AND storage_root == EMPTY_TRIE_HASH`.
//!   In that case the account node is deleted from the state trie rather than stored.
//!
//! This matches EVM account deletion semantics (EIP-161 empty-account removal)
//! and avoids any spec ambiguity. Phase 6 Task 6.2 step 2g is implemented against this rule.
//!
//! Phase 6 adds `apply_bal` and its helpers in this file.

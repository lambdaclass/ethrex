//! This module contains the logic for the healing phase of snap sync
//! Healing is used to fix inconsistencies with the canonical trie
//! The reason for these inconsistencies is that state download can spawn across multiple sync cycles each with a different pivot block,
//! meaning that the resulting trie is made up of fragments of different state tries and is not consistent with any block's state trie
//! 
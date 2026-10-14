use std::{collections::HashSet, hash::BuildHasher};

use ethrex_common::{Address, U256};
use ethrex_levm::environment::TransientStorage;
use rustc_hash::FxBuildHasher;

/// rustc-hash 2.x hashes a 32-byte key with a multiply whose first operand is
/// `limb[2] ^ 0x13198a2e03707344`, a public constant. Keys whose third limb is that
/// constant zero the product, so every other limb drops out of the hash.
const COLLIDING_LIMB: u64 = 0x13198a2e03707344;

fn colliding_keys() -> Vec<(Address, U256)> {
    let address = Address::from_low_u64_be(0x7777);
    (0..4096u64)
        .map(|i| (address, U256([i, 0, COLLIDING_LIMB, 0])))
        .collect()
}

fn distinct_hashes(hasher: &impl BuildHasher, keys: &[(Address, U256)]) -> usize {
    keys.iter()
        .map(|key| hasher.hash_one(key))
        .collect::<HashSet<_>>()
        .len()
}

#[test]
fn transient_storage_hashes_colliding_keys_apart() {
    let storage = TransientStorage::default();
    assert_eq!(distinct_hashes(storage.hasher(), &colliding_keys()), 4096);
}

#[test]
fn colliding_keys_share_one_fxhash() {
    // Shows the keys above are the attack shape. If this fails, rustc-hash changed
    // its constants: update the key shape, not the transient storage hasher.
    assert_eq!(distinct_hashes(&FxBuildHasher, &colliding_keys()), 1);
}

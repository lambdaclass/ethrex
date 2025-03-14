use ethereum_types::{Address, H256};

pub fn hash_to_address(hash: H256) -> Address {
    Address::from_slice(&hash.as_fixed_bytes()[12..])
}

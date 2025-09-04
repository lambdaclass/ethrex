use ethereum_types::Address;
use ethrex_rlp::encode::RLPEncode;
use sha3::{Digest, Keccak256};

/// Calculates the address of a new conctract using the CREATE
/// opcode as follows:
///
/// address = keccak256(rlp([sender_address,sender_nonce]))[12:]
/// TODO: This was copy-pasted from LEVM but without the internal error. Integrate them.
pub fn calculate_create_address(sender_address: Address, sender_nonce: u64) -> Address {
    let mut encoded = Vec::new();
    (sender_address, sender_nonce).encode(&mut encoded);
    let mut hasher = Keccak256::new();
    hasher.update(encoded);
    Address::from_slice(&hasher.finalize()[12..])
}

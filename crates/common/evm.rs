use ethereum_types::Address;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::RLPEncode;

/// Calculates the address of a new conctract using the CREATE
/// opcode as follows:
///
/// address = keccak256(rlp([sender_address,sender_nonce]))[12:]
pub fn calculate_create_address(sender_address: Address, sender_nonce: u64) -> Address {
    let mut encoded = Vec::new();
    (sender_address, sender_nonce).encode(&mut encoded);
    Address::from_slice(&keccak_hash(encoded)[12..])
}

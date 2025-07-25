pub mod constants;
pub mod decode;
pub mod encode;
pub mod error;
pub mod structs;

use ethereum_types::H256;
use std::str::FromStr;

pub fn get_hashed_keys() -> Vec<&'static str> {
    include!("../../../hashed_keys.rs")
}

pub fn get_proofs() -> Vec<Vec<u8>> {
    include!("../../../proof.rs")
}

pub fn get_encoded_values() -> Vec<Vec<u8>> {
    include!("../../../values.rs")
}

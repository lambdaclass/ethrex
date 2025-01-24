use crate::{Address, H256, U256};
pub type AccessList = Vec<AccessListItem>;
pub type AccessListItem = (Address, Vec<H256>);

pub type AuthorizationList = Vec<AuthorizationTuple>;
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorizationTuple {
    pub chain_id: U256,
    pub address: Address,
    pub nonce: u64,
    pub v: U256,
    pub r_signature: U256,
    pub s_signature: U256,
}

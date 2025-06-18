use ethrex_common::types::{ChainConfig, Fork};
// pub use revm::primitives::SpecId;

use ethrex_common::Address;

pub fn create_contract_address(from: Address, nonce: u64) -> Address {
    // Address::from_slice(
    //     revm::primitives::Address(from.0.into())
    //         .create(nonce)
    //         .0
    //         .as_ref(),
    // )
    Address::zero()
}

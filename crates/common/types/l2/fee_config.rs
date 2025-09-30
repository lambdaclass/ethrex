use ethereum_types::Address;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

use crate::rkyv_utils::{H160Wrapper, OptionH160Wrapper};

#[derive(
    Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug, Default,
)]
pub struct FeeConfig {
    /// If set, the base fee is sent to this address instead of being burned.
    #[rkyv(with=OptionH160Wrapper)]
    pub fee_vault: Option<Address>,
    pub operator_fee_config: Option<OperatorFeeConfig>,
}

/// Configuration for operator fees on L2
/// The operator fee is an additional fee on top of the base fee
/// that is sent to the operator fee vault.
/// This is used to pay for the cost of running the L2 network.
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug)]
pub struct OperatorFeeConfig {
    #[rkyv(with=H160Wrapper)]
    pub operator_fee_vault: Address,
    pub operator_fee: u64,
}

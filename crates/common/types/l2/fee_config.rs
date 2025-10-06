use ethereum_types::{Address, U256};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

use crate::rkyv_utils::{H160Wrapper, OptionH160Wrapper, U256Wrapper};

#[derive(
    Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug, Default,
)]
pub struct FeeConfig {
    /// If set, the base fee is sent to this address instead of being burned.
    #[rkyv(with=OptionH160Wrapper)]
    pub base_fee_vault: Option<Address>,
    pub operator_fee_config: Option<OperatorFeeConfig>,
    pub l1_fee_config: Option<L1FeeConfig>,
}

/// Configuration for operator fees on L2
/// The operator fee is an additional fee on top of the base fee
/// that is sent to the operator fee vault.
/// This is used to pay for the cost of running the L2 network.
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug)]
pub struct OperatorFeeConfig {
    #[rkyv(with=H160Wrapper)]
    pub operator_fee_vault: Address,
    #[rkyv(with=U256Wrapper)]
    pub operator_fee: U256,
}

/// L1 Fee is used to pay for the cost of
/// posting data to L1 (e.g. blob data).
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug)]
pub struct L1FeeConfig {
    #[rkyv(with=H160Wrapper)]
    pub l1_fee_vault: Address,
    #[rkyv(with=U256Wrapper)]
    pub l1_fee_per_blob_byte: U256,
}

use super::payload::PayloadStatus;
use bytes::Bytes;
use ethrex_common::{Address, H256, serde_utils, types::Withdrawal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkChoiceState {
    #[allow(unused)]
    pub head_block_hash: H256,
    pub safe_block_hash: H256,
    pub finalized_block_hash: H256,
}

#[derive(Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct PayloadAttributesV3 {
    #[serde(with = "serde_utils::u64::hex_str")]
    pub timestamp: u64,
    pub prev_randao: H256,
    pub suggested_fee_recipient: Address,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub parent_beacon_block_root: Option<H256>,
}

#[derive(Debug, Deserialize, Default, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct PayloadAttributesV4 {
    #[serde(with = "serde_utils::u64::hex_str")]
    pub timestamp: u64,
    pub prev_randao: H256,
    pub suggested_fee_recipient: Address,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub parent_beacon_block_root: Option<H256>,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub slot_number: u64,
    // execution-apis#796: CL-supplied target gas limit for local payload
    // building. Required on V4; an absent field fails deserialization and the
    // FCUv4 request is rejected (see `parse_v4`).
    #[serde(with = "serde_utils::u64::hex_str")]
    pub target_gas_limit: u64,
}

/// EIP-7805 (FOCIL) payload attributes. A superset of [`PayloadAttributesV4`]:
/// FOCIL only adds the inclusion list the consensus layer wants the locally
/// built block to honour.
#[derive(Debug, Deserialize, Default, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct PayloadAttributesV5 {
    #[serde(with = "serde_utils::u64::hex_str")]
    pub timestamp: u64,
    pub prev_randao: H256,
    pub suggested_fee_recipient: Address,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub parent_beacon_block_root: Option<H256>,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub slot_number: u64,
    /// EIP-2718-encoded inclusion-list transactions, as received. Kept as raw
    /// bytes here because an entry that does not decode is tolerated rather
    /// than rejected (see `build_payload_v5`).
    #[serde(with = "serde_utils::bytes::vec")]
    pub inclusion_list_transactions: Vec<Bytes>,
    // execution-apis#796: CL-supplied target gas limit, carried forward from
    // V4. Required, as on V4: Bogotá is post-Amsterdam, where the gas target is
    // mandatory, so an absent field fails deserialization and the FCUv5 request
    // is rejected.
    #[serde(with = "serde_utils::u64::hex_str")]
    pub target_gas_limit: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkChoiceResponse {
    pub payload_status: PayloadStatus,
    #[serde(with = "serde_utils::u64::hex_str_opt_padded")]
    pub payload_id: Option<u64>,
}

impl ForkChoiceResponse {
    pub fn set_id(&mut self, id: u64) {
        self.payload_id = Some(id)
    }
}

impl From<PayloadStatus> for ForkChoiceResponse {
    fn from(value: PayloadStatus) -> Self {
        Self {
            payload_status: value,
            payload_id: None,
        }
    }
}

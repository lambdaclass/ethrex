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
    #[serde(with = "serde_utils::bytes::vec")]
    pub inclusion_list_transactions: Vec<Bytes>,
    // execution-apis#796: CL-supplied target gas limit, carried forward from
    // V4. V5 attributes are a superset of V4 (FOCIL only adds the IL), so the
    // gas target stays available on the FOCIL local-build path. Required (FOCIL
    // runs on Hegotá, post-Amsterdam, where the gas target is mandatory); an
    // absent field fails deserialization and the FCUv5 request is rejected,
    // matching V4.
    #[serde(with = "serde_utils::u64::hex_str")]
    pub target_gas_limit: u64,
}

impl From<&PayloadAttributesV5> for PayloadAttributesV4 {
    fn from(value: &PayloadAttributesV5) -> Self {
        Self {
            timestamp: value.timestamp,
            prev_randao: value.prev_randao,
            suggested_fee_recipient: value.suggested_fee_recipient,
            withdrawals: value.withdrawals.clone(),
            parent_beacon_block_root: value.parent_beacon_block_root,
            slot_number: value.slot_number,
            target_gas_limit: value.target_gas_limit,
        }
    }
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

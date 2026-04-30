use super::payload::PayloadStatus;
#[cfg(feature = "eip-7805")]
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
}

#[cfg(feature = "eip-7805")]
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
}

#[cfg(feature = "eip-7805")]
impl From<&PayloadAttributesV5> for PayloadAttributesV4 {
    fn from(value: &PayloadAttributesV5) -> Self {
        Self {
            timestamp: value.timestamp,
            prev_randao: value.prev_randao,
            suggested_fee_recipient: value.suggested_fee_recipient,
            withdrawals: value.withdrawals.clone(),
            parent_beacon_block_root: value.parent_beacon_block_root,
            slot_number: value.slot_number,
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

#[cfg(all(test, feature = "eip-7805"))]
mod tests {
    use super::*;

    #[test]
    fn payload_attributes_v5_round_trips_with_inclusion_list() {
        let json = r#"{
            "timestamp": "0x6846fb2",
            "prevRandao": "0x2971eefd1f71f3548728cad87c16cc91b979ef035054828c59a02e49ae300a84",
            "suggestedFeeRecipient": "0x8943545177806ed17b9f23f0a21ee5948ecaa776",
            "withdrawals": [],
            "parentBeaconBlockRoot": "0x4029a2342bb6d54db91457bc8e442be22b3481df8edea24cc721f9d0649f65be",
            "slotNumber": "0x10",
            "inclusionListTransactions": ["0xdeadbeef", "0x01020304"]
        }"#;

        let attrs: PayloadAttributesV5 =
            serde_json::from_str(json).expect("V5 attributes deserialize");

        assert_eq!(attrs.timestamp, 0x6846fb2);
        assert_eq!(attrs.slot_number, 0x10);
        assert_eq!(
            attrs.suggested_fee_recipient,
            Address::from_slice(
                &hex::decode("8943545177806ed17b9f23f0a21ee5948ecaa776")
                    .expect("decode fee recipient")
            )
        );
        assert!(attrs.withdrawals.is_some());
        assert!(attrs.parent_beacon_block_root.is_some());
        assert_eq!(attrs.inclusion_list_transactions.len(), 2);
        assert_eq!(
            attrs.inclusion_list_transactions[0].as_ref(),
            &[0xde, 0xad, 0xbe, 0xef][..]
        );
        assert_eq!(
            attrs.inclusion_list_transactions[1].as_ref(),
            &[0x01, 0x02, 0x03, 0x04][..]
        );

        let serialized = serde_json::to_string(&attrs).expect("V5 attributes serialize");
        assert!(
            serialized.contains("\"inclusionListTransactions\":[\"0xdeadbeef\",\"0x01020304\"]")
        );
        assert!(serialized.contains("\"slotNumber\":\"0x10\""));

        let reparsed: PayloadAttributesV5 =
            serde_json::from_str(&serialized).expect("V5 attributes round-trip");
        assert_eq!(
            reparsed.inclusion_list_transactions,
            attrs.inclusion_list_transactions
        );
        assert_eq!(reparsed.timestamp, attrs.timestamp);
        assert_eq!(reparsed.slot_number, attrs.slot_number);
    }

    #[test]
    fn payload_attributes_v5_accepts_empty_inclusion_list() {
        let json = r#"{
            "timestamp": "0x6846fb2",
            "prevRandao": "0x2971eefd1f71f3548728cad87c16cc91b979ef035054828c59a02e49ae300a84",
            "suggestedFeeRecipient": "0x8943545177806ed17b9f23f0a21ee5948ecaa776",
            "withdrawals": [],
            "parentBeaconBlockRoot": "0x4029a2342bb6d54db91457bc8e442be22b3481df8edea24cc721f9d0649f65be",
            "slotNumber": "0x10",
            "inclusionListTransactions": []
        }"#;

        let attrs: PayloadAttributesV5 =
            serde_json::from_str(json).expect("V5 attributes deserialize");
        assert!(attrs.inclusion_list_transactions.is_empty());

        let serialized = serde_json::to_string(&attrs).expect("V5 attributes serialize");
        assert!(serialized.contains("\"inclusionListTransactions\":[]"));
    }

    #[test]
    fn payload_attributes_v5_to_v4_drops_inclusion_list() {
        let attrs_v5 = PayloadAttributesV5 {
            timestamp: 0x6846fb2,
            prev_randao: H256::zero(),
            suggested_fee_recipient: Address::zero(),
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(H256::zero()),
            slot_number: 0x10,
            inclusion_list_transactions: vec![Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef])],
        };

        let attrs_v4: PayloadAttributesV4 = (&attrs_v5).into();
        assert_eq!(attrs_v4.timestamp, attrs_v5.timestamp);
        assert_eq!(attrs_v4.slot_number, attrs_v5.slot_number);
        assert_eq!(attrs_v4.prev_randao, attrs_v5.prev_randao);
        assert_eq!(
            attrs_v4.suggested_fee_recipient,
            attrs_v5.suggested_fee_recipient
        );
        assert_eq!(attrs_v4.withdrawals, attrs_v5.withdrawals);
        assert_eq!(
            attrs_v4.parent_beacon_block_root,
            attrs_v5.parent_beacon_block_root
        );
    }

    #[test]
    fn payload_attributes_v5_default_constructible() {
        let attrs = PayloadAttributesV5::default();
        assert_eq!(attrs.timestamp, 0);
        assert_eq!(attrs.slot_number, 0);
        assert!(attrs.inclusion_list_transactions.is_empty());
    }
}

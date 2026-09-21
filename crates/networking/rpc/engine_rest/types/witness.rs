//! Payload witness response from execution-apis #885 at e473f58911e49cc619fb4102d3973e4e049322f0.
//! These bounded REST lists deliberately differ from stateless guest containers.

use ethrex_common::H256;
use ethrex_common::types::{BlockHeader, block_execution_witness::RpcExecutionWitness};
use ethrex_rlp::decode::RLPDecode;
use libssz::SszEncode;
use libssz_derive::{SszDecode, SszEncode};
use libssz_types::{SszList, SszVector};

use super::common::{MAX_TRANSACTIONS_PER_PAYLOAD, PayloadStatus};
use crate::engine_rest::error::ProblemJson;

pub const MAX_WITNESS_ITEMS: usize = 1 << 20;
pub const MAX_WITNESS_ITEM_BYTES: usize = 1 << 20;
pub type WitnessItems = SszList<SszList<u8, MAX_WITNESS_ITEM_BYTES>, MAX_WITNESS_ITEMS>;
pub type PublicKeys = SszList<SszVector<u8, 65>, MAX_TRANSACTIONS_PER_PAYLOAD>;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionWitness {
    pub state: WitnessItems,
    pub codes: WitnessItems,
    pub headers: WitnessItems,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadStatusWithWitness {
    pub payload_status: PayloadStatus,
    pub witness: SszList<ExecutionWitness, 1>,
    pub public_keys: PublicKeys,
}

impl ExecutionWitness {
    /// Validate the ancestor chain even for cached witnesses. Do not sort a
    /// malformed cache entry into apparent validity or omit the parent header.
    pub(crate) fn from_rpc(
        witness: RpcExecutionWitness,
        parent: H256,
    ) -> Result<Self, ProblemJson> {
        if !(1..=256).contains(&witness.headers.len()) {
            return Err(ProblemJson::internal(
                "witness requires 1 to 256 ancestor headers",
            ));
        }
        let mut previous: Option<BlockHeader> = None;
        for bytes in &witness.headers {
            let header = BlockHeader::decode(bytes)
                .map_err(|e| ProblemJson::internal(&format!("invalid witness header: {e}")))?;
            if let Some(prev) = previous
                && (header.parent_hash != prev.hash()
                    || prev.number.checked_add(1) != Some(header.number))
            {
                return Err(ProblemJson::internal(
                    "witness headers are not a contiguous ancestor chain",
                ));
            }
            previous = Some(header);
        }
        if previous.as_ref().map(BlockHeader::hash) != Some(parent) {
            return Err(ProblemJson::internal(
                "witness does not end at the payload parent",
            ));
        }
        fn items(values: Vec<bytes::Bytes>) -> Result<WitnessItems, ProblemJson> {
            if values.len() > MAX_WITNESS_ITEMS {
                return Err(ProblemJson::internal(
                    "witness field exceeds MAX_WITNESS_ITEMS",
                ));
            }
            values
                .into_iter()
                .map(|bytes| {
                    bytes.to_vec().try_into().map_err(|_| {
                        ProblemJson::internal("witness item exceeds MAX_WITNESS_ITEM_BYTES")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| ProblemJson::internal("witness field exceeds MAX_WITNESS_ITEMS"))
        }
        Ok(Self {
            state: items(witness.state)?,
            codes: items(witness.codes)?,
            headers: items(witness.headers)?,
        })
    }
}

impl PayloadStatusWithWitness {
    /// SSZ offsets are uint32 even though the per-field list bounds permit a
    /// larger aggregate. Check before the encoder casts offsets to u32.
    pub(crate) fn check_encoded_length(&self) -> Result<(), ProblemJson> {
        if self.encoded_len() > u32::MAX as usize {
            return Err(ProblemJson::internal(
                "witness response exceeds SSZ offset range",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_rest::types::common::to_optional;
    use ethrex_rlp::encode::RLPEncode;
    use libssz::SszDecode;

    #[test]
    fn witness_response_matches_independent_wire_bytes() {
        let response = PayloadStatusWithWitness {
            payload_status: PayloadStatus::new(0, Some([0xaa; 32]), None),
            witness: to_optional(Some(ExecutionWitness {
                state: vec![vec![0xc0].try_into().unwrap()].try_into().unwrap(),
                codes: Default::default(),
                headers: vec![vec![0x01, 0x02].try_into().unwrap()]
                    .try_into()
                    .unwrap(),
            })),
            public_keys: vec![
                vec![4; 65].try_into().unwrap(),
                vec![5; 65].try_into().unwrap(),
            ]
            .try_into()
            .unwrap(),
        };
        // Outer offsets: 12, 12+41, 12+41+4+23. Optional witness has its own offset.
        let mut expected = vec![
            12, 0, 0, 0, 53, 0, 0, 0, 80, 0, 0, 0, 0, 9, 0, 0, 0, 41, 0, 0, 0,
        ];
        expected.extend([0xaa; 32]);
        expected.extend([
            4, 0, 0, 0, 12, 0, 0, 0, 17, 0, 0, 0, 17, 0, 0, 0, 4, 0, 0, 0, 0xc0, 4, 0, 0, 0, 1, 2,
        ]);
        expected.extend([4; 65]);
        expected.extend([5; 65]);
        assert_eq!(response.to_ssz(), expected);
        assert_eq!(
            PayloadStatusWithWitness::from_ssz_bytes(&expected).unwrap(),
            response
        );
        for index in [0, 4, 8, 53] {
            let mut malformed = expected.clone();
            malformed[index] = 255;
            assert!(PayloadStatusWithWitness::from_ssz_bytes(&malformed).is_err());
        }
        expected.pop(); // fixed-size keys cannot be truncated
        assert!(PayloadStatusWithWitness::from_ssz_bytes(&expected).is_err());
    }

    #[test]
    fn absent_witness_and_keys_have_no_selector_or_padding() {
        for status in [1, 2, 3] {
            let response = PayloadStatusWithWitness {
                payload_status: PayloadStatus::new(status, None, None),
                witness: Default::default(),
                public_keys: Default::default(),
            };
            let expected = vec![
                12, 0, 0, 0, 21, 0, 0, 0, 21, 0, 0, 0, status, 9, 0, 0, 0, 9, 0, 0, 0,
            ];
            assert_eq!(response.to_ssz(), expected);
        }
    }

    #[test]
    fn witness_bounds_reject_oversized_items_and_lists() {
        assert!(
            SszList::<u8, MAX_WITNESS_ITEM_BYTES>::from_ssz_bytes(&vec![
                0;
                MAX_WITNESS_ITEM_BYTES + 1
            ])
            .is_err()
        );
        assert!(WitnessItems::try_from(vec![SszList::default(); MAX_WITNESS_ITEMS + 1]).is_err());
        assert!(
            PublicKeys::from_ssz_bytes(&vec![0; 65 * (MAX_TRANSACTIONS_PER_PAYLOAD + 1)]).is_err()
        );
        let mut rpc = RpcExecutionWitness::default();
        let parent = BlockHeader::default();
        rpc.headers.push(parent.encode_to_vec().into());
        rpc.codes.push(vec![0; MAX_WITNESS_ITEM_BYTES + 1].into());
        assert!(ExecutionWitness::from_rpc(rpc, parent.hash()).is_err());
    }

    #[test]
    fn validates_header_count_order_linkage_and_parent() {
        let older = BlockHeader::default();
        let parent = BlockHeader {
            number: 1,
            parent_hash: older.hash(),
            ..Default::default()
        };
        let good = vec![older.encode_to_vec().into(), parent.encode_to_vec().into()];
        let make = |headers| RpcExecutionWitness {
            headers,
            ..Default::default()
        };
        assert!(ExecutionWitness::from_rpc(make(good.clone()), parent.hash()).is_ok());
        assert!(
            ExecutionWitness::from_rpc(make(vec![parent.encode_to_vec().into()]), parent.hash())
                .is_ok()
        );
        for headers in [
            vec![],
            vec![older.encode_to_vec().into(); 257],
            vec![vec![0xff].into()],
            vec![good[1].clone(), good[0].clone()],
            vec![good[0].clone(), good[0].clone()],
        ] {
            assert!(ExecutionWitness::from_rpc(make(headers), parent.hash()).is_err());
        }
        assert!(ExecutionWitness::from_rpc(make(good), H256::zero()).is_err());
    }
}

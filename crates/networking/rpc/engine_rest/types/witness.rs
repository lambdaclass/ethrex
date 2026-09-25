//! Payload witness response from execution-apis #885 at 40924d49a7edecebe4ebc430042d1d6f95a86b8a.
//! These bounded REST lists deliberately differ from stateless guest containers.

use ethrex_common::H256;
use ethrex_common::types::{BlockHeader, block_execution_witness::RpcExecutionWitness};
use ethrex_rlp::decode::RLPDecode;
use libssz::SszEncode;
use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use super::common::PayloadStatus;
use crate::engine_rest::error::ProblemJson;

pub const MAX_WITNESS_ITEMS: usize = 1 << 20;
pub const MAX_BYTES_PER_WITNESS_NODE: usize = 1 << 10;
pub const MAX_BYTES_PER_CODE: usize = 1 << 16;
pub const MAX_BYTES_PER_HEADER: usize = 1 << 10;
pub const MAX_WITNESS_HEADERS: usize = 1 << 8;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionWitness {
    pub state: SszList<SszList<u8, MAX_BYTES_PER_WITNESS_NODE>, MAX_WITNESS_ITEMS>,
    pub codes: SszList<SszList<u8, MAX_BYTES_PER_CODE>, MAX_WITNESS_ITEMS>,
    pub headers: SszList<SszList<u8, MAX_BYTES_PER_HEADER>, MAX_WITNESS_HEADERS>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadStatusWithWitness {
    pub payload_status: PayloadStatus,
    pub witness: SszList<ExecutionWitness, 1>,
}

impl ExecutionWitness {
    /// Validate the ancestor chain even for cached witnesses. Do not sort a
    /// malformed cache entry into apparent validity or omit the parent header.
    pub(crate) fn from_rpc(
        witness: RpcExecutionWitness,
        parent: H256,
    ) -> Result<Self, ProblemJson> {
        if !(1..=MAX_WITNESS_HEADERS).contains(&witness.headers.len()) {
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
        fn items<const MAX_BYTES: usize, const MAX_ITEMS: usize>(
            values: Vec<bytes::Bytes>,
        ) -> Result<SszList<SszList<u8, MAX_BYTES>, MAX_ITEMS>, ProblemJson> {
            if values.len() > MAX_ITEMS {
                return Err(ProblemJson::internal(
                    "witness field exceeds item count limit",
                ));
            }
            values
                .into_iter()
                .map(|bytes| {
                    bytes.to_vec().try_into().map_err(|_| {
                        ProblemJson::internal("witness item exceeds byte length limit")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| ProblemJson::internal("witness field exceeds item count limit"))
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
        };
        // Outer offsets: 8, 8+41. Optional witness has its own offset.
        let mut expected = vec![8, 0, 0, 0, 49, 0, 0, 0, 0, 9, 0, 0, 0, 41, 0, 0, 0];
        expected.extend([0xaa; 32]);
        expected.extend([
            4, 0, 0, 0, 12, 0, 0, 0, 17, 0, 0, 0, 17, 0, 0, 0, 4, 0, 0, 0, 0xc0, 4, 0, 0, 0, 1, 2,
        ]);
        assert_eq!(response.to_ssz(), expected);
        assert_eq!(
            PayloadStatusWithWitness::from_ssz_bytes(&expected).unwrap(),
            response
        );
        for index in [0, 4, 9, 13, 49] {
            let mut malformed = expected.clone();
            malformed[index] = 255;
            assert!(PayloadStatusWithWitness::from_ssz_bytes(&malformed).is_err());
        }
    }

    #[test]
    fn absent_witness_has_no_selector_or_padding() {
        for status in [1, 2, 3] {
            let response = PayloadStatusWithWitness {
                payload_status: PayloadStatus::new(status, None, None),
                witness: Default::default(),
            };
            let expected = vec![8, 0, 0, 0, 17, 0, 0, 0, status, 9, 0, 0, 0, 9, 0, 0, 0];
            assert_eq!(response.to_ssz(), expected);
            assert_eq!(
                PayloadStatusWithWitness::from_ssz_bytes(&expected).unwrap(),
                response
            );
        }
    }

    #[test]
    fn witness_bounds_reject_oversized_items_and_lists() {
        // Construct the wire bytes independently, including an over-limit field
        // that cannot be constructed through SszList's checked API.
        let encode = |state: Vec<Vec<u8>>, codes: Vec<Vec<u8>>, headers: Vec<Vec<u8>>| {
            let fields = [state.to_ssz(), codes.to_ssz(), headers.to_ssz()];
            let mut bytes = Vec::new();
            let mut offset = 12u32;
            for field in &fields {
                bytes.extend(offset.to_le_bytes());
                offset += field.len() as u32;
            }
            for field in fields {
                bytes.extend(field);
            }
            bytes
        };
        for extra in [0, 1] {
            let state = vec![vec![0; MAX_BYTES_PER_WITNESS_NODE + extra]];
            let codes = vec![vec![0; MAX_BYTES_PER_CODE + extra]];
            let headers = vec![vec![0; MAX_BYTES_PER_HEADER + extra]];
            for bytes in [
                encode(state, vec![], vec![]),
                encode(vec![], codes, vec![]),
                encode(vec![], vec![], headers),
                encode(vec![vec![]; MAX_WITNESS_ITEMS + extra], vec![], vec![]),
                encode(vec![], vec![vec![]; MAX_WITNESS_ITEMS + extra], vec![]),
                encode(vec![], vec![], vec![vec![]; MAX_WITNESS_HEADERS + extra]),
            ] {
                assert_eq!(ExecutionWitness::from_ssz_bytes(&bytes).is_ok(), extra == 0);
            }
        }
    }

    #[test]
    fn rpc_conversion_enforces_field_byte_limits() {
        let parent = BlockHeader::default();
        for extra in [0, 1] {
            for (state, codes) in [
                (
                    vec![vec![0; MAX_BYTES_PER_WITNESS_NODE + extra].into()],
                    vec![],
                ),
                (vec![], vec![vec![0; MAX_BYTES_PER_CODE + extra].into()]),
            ] {
                let rpc = RpcExecutionWitness {
                    state,
                    codes,
                    headers: vec![parent.encode_to_vec().into()],
                    ..Default::default()
                };
                assert_eq!(
                    ExecutionWitness::from_rpc(rpc, parent.hash()).is_ok(),
                    extra == 0
                );
            }
        }
        let oversized_parent = BlockHeader {
            extra_data: vec![0; MAX_BYTES_PER_HEADER].into(),
            ..Default::default()
        };
        let rpc = RpcExecutionWitness {
            headers: vec![oversized_parent.encode_to_vec().into()],
            ..Default::default()
        };
        assert!(ExecutionWitness::from_rpc(rpc, oversized_parent.hash()).is_err());
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

//! Forkchoice V1..V4 request containers.
//!
//! `payload_attributes` uses nullable encoding `List[T, 1]`: 0 elements when
//! no payload build is requested, 1 element otherwise.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::ForkchoiceStateV1;
use crate::engine_rest::types::payload_attributes::{
    PayloadAttributesV1, PayloadAttributesV2, PayloadAttributesV3, PayloadAttributesV4,
};

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ForkchoiceUpdatedV1Request {
    pub forkchoice_state: ForkchoiceStateV1,
    pub payload_attributes: SszList<PayloadAttributesV1, 1>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ForkchoiceUpdatedV2Request {
    pub forkchoice_state: ForkchoiceStateV1,
    pub payload_attributes: SszList<PayloadAttributesV2, 1>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ForkchoiceUpdatedV3Request {
    pub forkchoice_state: ForkchoiceStateV1,
    pub payload_attributes: SszList<PayloadAttributesV3, 1>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ForkchoiceUpdatedV4Request {
    pub forkchoice_state: ForkchoiceStateV1,
    pub payload_attributes: SszList<PayloadAttributesV4, 1>,
}

//! ExchangeCapabilities request/response.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::{MAX_CAPABILITIES, MAX_CAPABILITY_NAME_LENGTH};

pub type CapabilityList = SszList<SszList<u8, MAX_CAPABILITY_NAME_LENGTH>, MAX_CAPABILITIES>;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExchangeCapabilitiesRequest {
    pub capabilities: CapabilityList,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExchangeCapabilitiesResponse {
    pub capabilities: CapabilityList,
}

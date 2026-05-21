//! Client version request/response.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::{
    Bytes4, MAX_CLIENT_CODE_LENGTH, MAX_CLIENT_NAME_LENGTH, MAX_CLIENT_VERSION_LENGTH,
    MAX_CLIENT_VERSIONS,
};

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ClientVersionV1 {
    pub code: SszList<u8, MAX_CLIENT_CODE_LENGTH>,
    pub name: SszList<u8, MAX_CLIENT_NAME_LENGTH>,
    pub version: SszList<u8, MAX_CLIENT_VERSION_LENGTH>,
    pub commit: Bytes4,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetClientVersionV1Request {
    pub client_version: ClientVersionV1,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetClientVersionV1Response {
    pub versions: SszList<ClientVersionV1, MAX_CLIENT_VERSIONS>,
}

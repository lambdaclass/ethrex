use crate::constants::*;

use ethrex_core::{types::TxKind, Address, H256, U256};

use revm_primitives::SpecId;

/// After EIP-7691 the maximum number of blob hashes changes. For more
/// information see
/// [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691#specification).
pub const fn max_blobs_per_block(specid: SpecId) -> usize {
    match specid {
        SpecId::PRAGUE => MAX_BLOB_COUNT_ELECTRA,
        SpecId::PRAGUE_EOF => MAX_BLOB_COUNT_ELECTRA,
        _ => MAX_BLOB_COUNT,
    }
}

/// According to EIP-7691
/// (https://eips.ethereum.org/EIPS/eip-7691#specification):
///
/// "These changes imply that get_base_fee_per_blob_gas and
/// calc_excess_blob_gas functions defined in EIP-4844 use the new
/// values for the first block of the fork (and for all subsequent
/// blocks)."
pub const fn get_blob_base_fee_update_fraction_value(specid: SpecId) -> U256 {
    match specid {
        SpecId::PRAGUE => BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        SpecId::PRAGUE_EOF => BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        _ => BLOB_BASE_FEE_UPDATE_FRACTION,
    }
}

//! Type conversions between ethrex types and alloy/revm types (used by pevm)
//!
//! ethrex uses `ethereum_types` crate (Address, H256, U256)
//! pevm/alloy uses `alloy_primitives` (Address, B256, U256)
//! revm uses its own primitives for TxEnv and BlockEnv

use alloy_primitives::{Address as AlloyAddress, B256, Bytes as AlloyBytes};
use alloy_primitives::{Log as AlloyLog, U256 as AlloyU256};
use ethrex_common::types::{BlockHeader, Log, Receipt, Transaction, TxKind, TxType};
use ethrex_common::{Address, H256, U256};
use revm::primitives::{BlockEnv, TxEnv, TransactTo, AuthorizationList};

// =============================================================================
// Primitive Type Conversions
// =============================================================================

/// Convert ethrex Address to alloy Address (20 bytes)
pub fn ethrex_addr_to_alloy(addr: &Address) -> AlloyAddress {
    AlloyAddress::from_slice(addr.as_bytes())
}

/// Convert alloy Address to ethrex Address
pub fn alloy_addr_to_ethrex(addr: &AlloyAddress) -> Address {
    Address::from_slice(addr.as_slice())
}

/// Convert ethrex H256 to alloy B256 (32 bytes)
pub fn ethrex_h256_to_alloy(h: &H256) -> B256 {
    B256::from_slice(h.as_bytes())
}

/// Convert alloy B256 to ethrex H256
pub fn alloy_b256_to_ethrex(b: &B256) -> H256 {
    H256::from_slice(b.as_slice())
}

/// Convert ethrex U256 to alloy U256
pub fn ethrex_u256_to_alloy(u: &U256) -> AlloyU256 {
    AlloyU256::from_limbs(u.0)
}

/// Convert alloy U256 to ethrex U256
pub fn alloy_u256_to_ethrex(u: &AlloyU256) -> U256 {
    U256(u.into_limbs())
}

/// Convert ethrex Bytes to alloy Bytes
pub fn ethrex_bytes_to_alloy(b: &bytes::Bytes) -> AlloyBytes {
    AlloyBytes::copy_from_slice(b.as_ref())
}

/// Convert alloy Bytes to ethrex Bytes
pub fn alloy_bytes_to_ethrex(b: &AlloyBytes) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(b.as_ref())
}

// =============================================================================
// Block Environment Conversion (for revm)
// =============================================================================

/// Convert ethrex BlockHeader to revm BlockEnv
pub fn convert_block_env(header: &BlockHeader) -> BlockEnv {
    BlockEnv {
        number: AlloyU256::from(header.number),
        coinbase: ethrex_addr_to_alloy(&header.coinbase),
        timestamp: AlloyU256::from(header.timestamp),
        gas_limit: AlloyU256::from(header.gas_limit),
        basefee: AlloyU256::from(header.base_fee_per_gas.unwrap_or(0)),
        difficulty: ethrex_u256_to_alloy(&header.difficulty),
        prevrandao: Some(ethrex_h256_to_alloy(&header.prev_randao)),
        blob_excess_gas_and_price: header.excess_blob_gas.map(|excess| {
            revm::primitives::BlobExcessGasAndPrice::new(excess, false)
        }),
    }
}

// =============================================================================
// Transaction Environment Conversion (for revm)
// =============================================================================

/// Convert ethrex Transaction to revm TxEnv
pub fn convert_tx_env(tx: &Transaction, sender: AlloyAddress) -> Result<TxEnv, String> {
    match tx {
        Transaction::LegacyTransaction(legacy) => {
            let transact_to = match &legacy.to {
                TxKind::Call(addr) => TransactTo::Call(ethrex_addr_to_alloy(addr)),
                TxKind::Create => TransactTo::Create,
            };

            Ok(TxEnv {
                caller: sender,
                gas_limit: legacy.gas,
                gas_price: ethrex_u256_to_alloy(&legacy.gas_price),
                transact_to,
                value: ethrex_u256_to_alloy(&legacy.value),
                data: ethrex_bytes_to_alloy(&legacy.data),
                nonce: Some(legacy.nonce),
                chain_id: None,
                access_list: vec![],
                gas_priority_fee: None,
                blob_hashes: vec![],
                max_fee_per_blob_gas: None,
                authorization_list: None,
            })
        }

        Transaction::EIP2930Transaction(eip2930) => {
            let transact_to = match &eip2930.to {
                TxKind::Call(addr) => TransactTo::Call(ethrex_addr_to_alloy(addr)),
                TxKind::Create => TransactTo::Create,
            };

            let access_list = eip2930
                .access_list
                .iter()
                .map(|(addr, keys)| revm::primitives::AccessListItem {
                    address: ethrex_addr_to_alloy(addr),
                    storage_keys: keys.iter().map(ethrex_h256_to_alloy).collect(),
                })
                .collect();

            Ok(TxEnv {
                caller: sender,
                gas_limit: eip2930.gas_limit,
                gas_price: ethrex_u256_to_alloy(&eip2930.gas_price),
                transact_to,
                value: ethrex_u256_to_alloy(&eip2930.value),
                data: ethrex_bytes_to_alloy(&eip2930.data),
                nonce: Some(eip2930.nonce),
                chain_id: Some(eip2930.chain_id),
                access_list,
                gas_priority_fee: None,
                blob_hashes: vec![],
                max_fee_per_blob_gas: None,
                authorization_list: None,
            })
        }

        Transaction::EIP1559Transaction(eip1559) => {
            let transact_to = match &eip1559.to {
                TxKind::Call(addr) => TransactTo::Call(ethrex_addr_to_alloy(addr)),
                TxKind::Create => TransactTo::Create,
            };

            let access_list = eip1559
                .access_list
                .iter()
                .map(|(addr, keys)| revm::primitives::AccessListItem {
                    address: ethrex_addr_to_alloy(addr),
                    storage_keys: keys.iter().map(ethrex_h256_to_alloy).collect(),
                })
                .collect();

            Ok(TxEnv {
                caller: sender,
                gas_limit: eip1559.gas_limit,
                gas_price: AlloyU256::from(eip1559.max_fee_per_gas),
                transact_to,
                value: ethrex_u256_to_alloy(&eip1559.value),
                data: ethrex_bytes_to_alloy(&eip1559.data),
                nonce: Some(eip1559.nonce),
                chain_id: Some(eip1559.chain_id),
                access_list,
                gas_priority_fee: Some(AlloyU256::from(eip1559.max_priority_fee_per_gas)),
                blob_hashes: vec![],
                max_fee_per_blob_gas: None,
                authorization_list: None,
            })
        }

        Transaction::EIP4844Transaction(eip4844) => {
            let access_list = eip4844
                .access_list
                .iter()
                .map(|(addr, keys)| revm::primitives::AccessListItem {
                    address: ethrex_addr_to_alloy(addr),
                    storage_keys: keys.iter().map(ethrex_h256_to_alloy).collect(),
                })
                .collect();

            Ok(TxEnv {
                caller: sender,
                gas_limit: eip4844.gas,
                gas_price: AlloyU256::from(eip4844.max_fee_per_gas),
                transact_to: TransactTo::Call(ethrex_addr_to_alloy(&eip4844.to)),
                value: ethrex_u256_to_alloy(&eip4844.value),
                data: ethrex_bytes_to_alloy(&eip4844.data),
                nonce: Some(eip4844.nonce),
                chain_id: Some(eip4844.chain_id),
                access_list,
                gas_priority_fee: Some(AlloyU256::from(eip4844.max_priority_fee_per_gas)),
                blob_hashes: eip4844
                    .blob_versioned_hashes
                    .iter()
                    .map(ethrex_h256_to_alloy)
                    .collect(),
                max_fee_per_blob_gas: Some(ethrex_u256_to_alloy(&eip4844.max_fee_per_blob_gas)),
                authorization_list: None,
            })
        }

        Transaction::EIP7702Transaction(eip7702) => {
            let access_list = eip7702
                .access_list
                .iter()
                .map(|(addr, keys)| revm::primitives::AccessListItem {
                    address: ethrex_addr_to_alloy(addr),
                    storage_keys: keys.iter().map(ethrex_h256_to_alloy).collect(),
                })
                .collect();

            let auth_list: Vec<revm::primitives::SignedAuthorization> = eip7702
                .authorization_list
                .iter()
                .map(|auth| {
                    revm::primitives::SignedAuthorization::new_unchecked(
                        revm::primitives::Authorization {
                            chain_id: ethrex_u256_to_alloy(&auth.chain_id),
                            address: ethrex_addr_to_alloy(&auth.address),
                            nonce: auth.nonce,
                        },
                        auth.y_parity.as_u32() as u8,
                        ethrex_u256_to_alloy(&auth.r_signature),
                        ethrex_u256_to_alloy(&auth.s_signature),
                    )
                })
                .collect();

            Ok(TxEnv {
                caller: sender,
                gas_limit: eip7702.gas_limit,
                gas_price: AlloyU256::from(eip7702.max_fee_per_gas),
                transact_to: TransactTo::Call(ethrex_addr_to_alloy(&eip7702.to)),
                value: ethrex_u256_to_alloy(&eip7702.value),
                data: ethrex_bytes_to_alloy(&eip7702.data),
                nonce: Some(eip7702.nonce),
                chain_id: Some(eip7702.chain_id),
                access_list,
                gas_priority_fee: Some(AlloyU256::from(eip7702.max_priority_fee_per_gas)),
                blob_hashes: vec![],
                max_fee_per_blob_gas: None,
                authorization_list: Some(AuthorizationList::Signed(auth_list)),
            })
        }

        // L2 transactions are not supported in pevm backend
        Transaction::PrivilegedL2Transaction(_) => {
            Err("PrivilegedL2Transaction not supported in pevm backend".to_string())
        }
        Transaction::FeeTokenTransaction(_) => {
            Err("FeeTokenTransaction not supported in pevm backend".to_string())
        }
    }
}

// =============================================================================
// Result Conversion (pevm -> ethrex)
// =============================================================================

/// Convert alloy Log to ethrex Log
pub fn convert_log_to_ethrex(log: &AlloyLog) -> Log {
    Log {
        address: alloy_addr_to_ethrex(&log.address),
        topics: log.topics().iter().map(alloy_b256_to_ethrex).collect(),
        data: bytes::Bytes::copy_from_slice(log.data.data.as_ref()),
    }
}

/// Convert pevm execution logs to ethrex logs
pub fn convert_logs_to_ethrex(logs: &[AlloyLog]) -> Vec<Log> {
    logs.iter().map(convert_log_to_ethrex).collect()
}

/// Create an ethrex Receipt from pevm execution result
pub fn create_receipt(
    tx_type: TxType,
    succeeded: bool,
    cumulative_gas_used: u64,
    logs: Vec<Log>,
) -> Receipt {
    Receipt::new(tx_type, succeeded, cumulative_gas_used, logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_roundtrip() {
        let ethrex_addr = Address::from_low_u64_be(0x1234);
        let alloy_addr = ethrex_addr_to_alloy(&ethrex_addr);
        let back = alloy_addr_to_ethrex(&alloy_addr);
        assert_eq!(ethrex_addr, back);
    }

    #[test]
    fn test_h256_roundtrip() {
        let ethrex_h = H256::from_low_u64_be(0x5678);
        let alloy_b = ethrex_h256_to_alloy(&ethrex_h);
        let back = alloy_b256_to_ethrex(&alloy_b);
        assert_eq!(ethrex_h, back);
    }

    #[test]
    fn test_u256_roundtrip() {
        let ethrex_u = U256::from(0x9abc_u64);
        let alloy_u = ethrex_u256_to_alloy(&ethrex_u);
        let back = alloy_u256_to_ethrex(&alloy_u);
        assert_eq!(ethrex_u, back);
    }
}

pub fn migrate_block_header(
    header: ethrex_common_libmdbx::types::BlockHeader,
) -> ethrex_common::types::BlockHeader {
    ethrex_common::types::BlockHeader {
        hash: header.hash,
        parent_hash: header.parent_hash,
        ommers_hash: header.ommers_hash,
        coinbase: header.coinbase,
        state_root: header.state_root,
        transactions_root: header.transactions_root,
        receipts_root: header.receipts_root,
        logs_bloom: header.logs_bloom,
        difficulty: header.difficulty,
        number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        extra_data: header.extra_data,
        prev_randao: header.prev_randao,
        nonce: header.nonce,
        base_fee_per_gas: header.base_fee_per_gas,
        withdrawals_root: header.withdrawals_root,
        blob_gas_used: header.blob_gas_used,
        excess_blob_gas: header.excess_blob_gas,
        parent_beacon_block_root: header.parent_beacon_block_root,
        requests_hash: header.requests_hash,
    }
}

pub fn migrate_block_body(
    body: ethrex_common_libmdbx::types::BlockBody,
) -> ethrex_common::types::BlockBody {
    ethrex_common::types::BlockBody {
        transactions: body
            .transactions
            .iter()
            .map(|tx| migrate_transaction(tx.clone()))
            .collect(),
        ommers: body
            .ommers
            .iter()
            .map(|ommer| migrate_block_header(ommer.clone()))
            .collect(),
        withdrawals: body.withdrawals.map(|withdrawals| {
            withdrawals
                .iter()
                .map(|withdrawal| ethrex_common::types::Withdrawal {
                    index: withdrawal.index,
                    validator_index: withdrawal.validator_index,
                    address: withdrawal.address,
                    amount: withdrawal.amount,
                })
                .collect()
        }),
    }
}

pub fn migrate_transaction(
    tx: ethrex_common_libmdbx::types::Transaction,
) -> ethrex_common::types::Transaction {
    match tx {
        ethrex_common_libmdbx::types::Transaction::EIP1559Transaction(tx) => {
            ethrex_common::types::Transaction::EIP1559Transaction(
                ethrex_common::types::EIP1559Transaction {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce,
                    max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                    max_fee_per_gas: tx.max_fee_per_gas,
                    gas_limit: tx.gas_limit,
                    to: match tx.to {
                        ethrex_common_libmdbx::types::TxKind::Create => {
                            ethrex_common::types::TxKind::Create
                        }
                        ethrex_common_libmdbx::types::TxKind::Call(to) => {
                            ethrex_common::types::TxKind::Call(to)
                        }
                    },
                    value: tx.value,
                    data: tx.data,
                    access_list: tx.access_list,
                    signature_y_parity: tx.signature_y_parity,
                    signature_r: tx.signature_r,
                    signature_s: tx.signature_s,
                    inner_hash: tx.inner_hash,
                },
            )
        }
        ethrex_common_libmdbx::types::Transaction::LegacyTransaction(tx) => {
            ethrex_common::types::Transaction::LegacyTransaction(
                ethrex_common::types::LegacyTransaction {
                    nonce: tx.nonce,
                    gas_price: tx.gas_price,
                    gas: tx.gas,
                    to: match tx.to {
                        ethrex_common_libmdbx::types::TxKind::Create => {
                            ethrex_common::types::TxKind::Create
                        }
                        ethrex_common_libmdbx::types::TxKind::Call(to) => {
                            ethrex_common::types::TxKind::Call(to)
                        }
                    },
                    value: tx.value,
                    data: tx.data,
                    v: tx.v,
                    r: tx.r,
                    s: tx.s,
                    inner_hash: tx.inner_hash,
                },
            )
        }
        ethrex_common_libmdbx::types::Transaction::EIP2930Transaction(tx) => {
            ethrex_common::types::Transaction::EIP2930Transaction(
                ethrex_common::types::EIP2930Transaction {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce,
                    gas_price: tx.gas_price,
                    gas_limit: tx.gas_limit,
                    to: match tx.to {
                        ethrex_common_libmdbx::types::TxKind::Create => {
                            ethrex_common::types::TxKind::Create
                        }
                        ethrex_common_libmdbx::types::TxKind::Call(to) => {
                            ethrex_common::types::TxKind::Call(to)
                        }
                    },
                    value: tx.value,
                    data: tx.data,
                    access_list: tx.access_list,
                    signature_y_parity: tx.signature_y_parity,
                    signature_r: tx.signature_r,
                    signature_s: tx.signature_s,
                    inner_hash: tx.inner_hash,
                },
            )
        }
        ethrex_common_libmdbx::types::Transaction::EIP4844Transaction(tx) => {
            ethrex_common::types::Transaction::EIP4844Transaction(
                ethrex_common::types::EIP4844Transaction {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce,
                    max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                    max_fee_per_gas: tx.max_fee_per_gas,
                    gas: tx.gas,
                    to: tx.to,
                    value: tx.value,
                    data: tx.data,
                    access_list: tx.access_list,
                    max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
                    blob_versioned_hashes: tx.blob_versioned_hashes,
                    signature_y_parity: tx.signature_y_parity,
                    signature_r: tx.signature_r,
                    signature_s: tx.signature_s,
                    inner_hash: tx.inner_hash,
                },
            )
        }
        ethrex_common_libmdbx::types::Transaction::EIP7702Transaction(tx) => {
            ethrex_common::types::Transaction::EIP7702Transaction(
                ethrex_common::types::EIP7702Transaction {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce,
                    max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                    max_fee_per_gas: tx.max_fee_per_gas,
                    gas_limit: tx.gas_limit,
                    to: tx.to,
                    value: tx.value,
                    data: tx.data,
                    access_list: tx.access_list,
                    authorization_list: tx
                        .authorization_list
                        .iter()
                        .map(|auth| ethrex_common::types::AuthorizationTuple {
                            chain_id: auth.chain_id,
                            address: auth.address,
                            nonce: auth.nonce,
                            y_parity: auth.y_parity,
                            r_signature: auth.r_signature,
                            s_signature: auth.s_signature,
                        })
                        .collect(),
                    signature_y_parity: tx.signature_y_parity,
                    signature_r: tx.signature_r,
                    signature_s: tx.signature_s,
                    inner_hash: tx.inner_hash,
                },
            )
        }
        ethrex_common_libmdbx::types::Transaction::PrivilegedL2Transaction(tx) => {
            ethrex_common::types::Transaction::PrivilegedL2Transaction(
                ethrex_common::types::PrivilegedL2Transaction {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce,
                    max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                    max_fee_per_gas: tx.max_fee_per_gas,
                    gas_limit: tx.gas_limit,
                    to: match tx.to {
                        ethrex_common_libmdbx::types::TxKind::Create => {
                            ethrex_common::types::TxKind::Create
                        }
                        ethrex_common_libmdbx::types::TxKind::Call(to) => {
                            ethrex_common::types::TxKind::Call(to)
                        }
                    },
                    value: tx.value,
                    data: tx.data,
                    access_list: tx.access_list,
                    from: tx.from,
                    inner_hash: tx.inner_hash,
                },
            )
        }
    }
}

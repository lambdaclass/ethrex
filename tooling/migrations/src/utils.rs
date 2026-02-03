use ethrex_common::types::{
    AuthorizationTuple, BlockBody, BlockHeader, EIP1559Transaction, EIP2930Transaction,
    EIP4844Transaction, EIP7702Transaction, LegacyTransaction, PrivilegedL2Transaction,
    Transaction, TxKind, Withdrawal,
};
use ethrex_common_libmdbx::types::{
    BlockBody as LibmdbxBlockBody, BlockHeader as LibmdbxBlockHeader,
    Transaction as LibmdbxTransaction, TxKind as LibmdbxTxKind,
};

pub fn migrate_block_header(header: LibmdbxBlockHeader) -> BlockHeader {
    BlockHeader {
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
        block_access_list_hash: None,
        slot_number: None,
    }
}

pub fn migrate_block_body(body: LibmdbxBlockBody) -> BlockBody {
    BlockBody {
        transactions: body
            .transactions
            .into_iter()
            .map(migrate_transaction)
            .collect(),
        ommers: body.ommers.into_iter().map(migrate_block_header).collect(),
        withdrawals: body.withdrawals.map(|withdrawals| {
            withdrawals
                .iter()
                .map(|withdrawal| Withdrawal {
                    index: withdrawal.index,
                    validator_index: withdrawal.validator_index,
                    address: withdrawal.address,
                    amount: withdrawal.amount,
                })
                .collect()
        }),
    }
}

pub fn migrate_transaction(tx: LibmdbxTransaction) -> Transaction {
    match tx {
        LibmdbxTransaction::EIP1559Transaction(tx) => {
            Transaction::EIP1559Transaction(EIP1559Transaction {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                max_fee_per_gas: tx.max_fee_per_gas,
                gas_limit: tx.gas_limit,
                to: match tx.to {
                    LibmdbxTxKind::Create => TxKind::Create,
                    LibmdbxTxKind::Call(to) => TxKind::Call(to),
                },
                value: tx.value,
                data: tx.data,
                access_list: tx.access_list,
                signature_y_parity: tx.signature_y_parity,
                signature_r: tx.signature_r,
                signature_s: tx.signature_s,
                inner_hash: tx.inner_hash,
            })
        }
        LibmdbxTransaction::LegacyTransaction(tx) => {
            Transaction::LegacyTransaction(LegacyTransaction {
                nonce: tx.nonce,
                gas_price: tx.gas_price.into(),
                gas: tx.gas,
                to: match tx.to {
                    LibmdbxTxKind::Create => TxKind::Create,
                    LibmdbxTxKind::Call(to) => TxKind::Call(to),
                },
                value: tx.value,
                data: tx.data,
                v: tx.v,
                r: tx.r,
                s: tx.s,
                inner_hash: tx.inner_hash,
            })
        }
        LibmdbxTransaction::EIP2930Transaction(tx) => {
            Transaction::EIP2930Transaction(EIP2930Transaction {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                gas_price: tx.gas_price.into(),
                gas_limit: tx.gas_limit,
                to: match tx.to {
                    LibmdbxTxKind::Create => TxKind::Create,
                    LibmdbxTxKind::Call(to) => TxKind::Call(to),
                },
                value: tx.value,
                data: tx.data,
                access_list: tx.access_list,
                signature_y_parity: tx.signature_y_parity,
                signature_r: tx.signature_r,
                signature_s: tx.signature_s,
                inner_hash: tx.inner_hash,
            })
        }
        LibmdbxTransaction::EIP4844Transaction(tx) => {
            Transaction::EIP4844Transaction(EIP4844Transaction {
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
            })
        }
        LibmdbxTransaction::EIP7702Transaction(tx) => {
            Transaction::EIP7702Transaction(EIP7702Transaction {
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
                    .map(|auth| AuthorizationTuple {
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
            })
        }
        LibmdbxTransaction::PrivilegedL2Transaction(tx) => {
            Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                max_fee_per_gas: tx.max_fee_per_gas,
                gas_limit: tx.gas_limit,
                to: match tx.to {
                    LibmdbxTxKind::Create => TxKind::Create,
                    LibmdbxTxKind::Call(to) => TxKind::Call(to),
                },
                value: tx.value,
                data: tx.data,
                access_list: tx.access_list,
                from: tx.from,
                inner_hash: tx.inner_hash,
            })
        }
    }
}

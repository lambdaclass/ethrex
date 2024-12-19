use bytes::BufMut;
use bytes::Bytes;
use ethrex_blockchain::error::MempoolError;
use ethrex_blockchain::mempool;
use ethrex_core::types::P2PTransaction;
use ethrex_core::types::WrappedEIP4844Transaction;
use ethrex_core::{types::Transaction, H256};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::{error::StoreError, Store};

use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
// Broadcast message
#[derive(Debug, Clone)]
pub(crate) struct Transactions {
    pub(crate) transactions: Vec<Transaction>,
}
// TODO(#1132): Also limit transactions by message byte-size.
// Limit taken from here: https://github.com/ethereum/go-ethereum/blob/df182a742cec68adcc034d4747afa5182fc75ca3/eth/fetcher/tx_fetcher.go#L49
pub const TRANSACTION_LIMIT: usize = 256;

impl Transactions {
    pub fn new(transactions: Vec<Transaction>) -> Self {
        Self { transactions }
    }
}

impl RLPxMessage for Transactions {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        let mut encoder = Encoder::new(&mut encoded_data);
        let txs_iter = self.transactions.iter();
        for tx in txs_iter {
            encoder = encoder.encode_field(tx)
        }
        encoder.finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let mut decoder = Decoder::new(&decompressed_data)?;
        let mut transactions: Vec<Transaction> = vec![];
        // This is done like this because the blanket Vec<T> implementation
        // gets confused since a legacy transaction is actually a list,
        // or so it seems.
        while let Ok((tx, updated_decoder)) = decoder.decode_field::<Transaction>("p2p transaction")
        {
            if transactions.len() > TRANSACTION_LIMIT {
                break;
            } else {
                decoder = updated_decoder;
                transactions.push(tx);
            }
        }
        Ok(Self::new(transactions))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x08
// Broadcast message
#[derive(Debug)]
pub(crate) struct NewPooledTransactionHashes {
    transaction_types: Bytes,
    transaction_sizes: Vec<usize>,
    transaction_hashes: Vec<H256>,
}

impl NewPooledTransactionHashes {
    // delete this after we use this in the main loop
    #[allow(dead_code)]
    pub fn new(transactions: Vec<Transaction>) -> Self {
        let transactions_len = transactions.len();
        let mut transaction_types = Vec::with_capacity(transactions_len);
        let mut transaction_sizes = Vec::with_capacity(transactions_len);
        let mut transaction_hashes = Vec::with_capacity(transactions_len);
        for transaction in transactions {
            let transaction_type = transaction.tx_type();
            transaction_types.push(transaction_type as u8);
            // size is defined as the len of the concatenation of tx_type and the tx_data
            // as the tx_type goes from 0x00 to 0xff, the size of tx_type is 1 byte
            let transaction_size = 1 + transaction.data().len();
            transaction_sizes.push(transaction_size);
            let transaction_hash = transaction.compute_hash();
            transaction_hashes.push(transaction_hash);
        }
        Self {
            transaction_types: transaction_types.into(),
            transaction_sizes,
            transaction_hashes,
        }
    }

    pub fn get_transactions_to_request(&self, storage: &Store) -> Result<Vec<H256>, StoreError> {
        storage.filter_unknown_transactions(&self.transaction_hashes)
    }
}

impl RLPxMessage for NewPooledTransactionHashes {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.transaction_types)
            .encode_field(&self.transaction_sizes)
            .encode_field(&self.transaction_hashes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (transaction_types, decoder): (Bytes, _) = decoder.decode_field("transactionTypes")?;
        let (transaction_sizes, decoder): (Vec<usize>, _) =
            decoder.decode_field("transactionSizes")?;
        let (transaction_hashes, _): (Vec<H256>, _) = decoder.decode_field("transactionHashes")?;

        if transaction_hashes.len() == transaction_sizes.len()
            && transaction_sizes.len() == transaction_types.len()
        {
            Ok(Self {
                transaction_types,
                transaction_sizes,
                transaction_hashes,
            })
        } else {
            Err(RLPDecodeError::Custom(
                "transaction_hashes, transaction_sizes and transaction_types must have the same length"
                    .to_string(),
            ))
        }
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getpooledtransactions-0x09
#[derive(Debug)]
pub(crate) struct GetPooledTransactions {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    id: u64,
    transaction_hashes: Vec<H256>,
}

impl GetPooledTransactions {
    pub fn new(id: u64, transaction_hashes: Vec<H256>) -> Self {
        Self {
            transaction_hashes,
            id,
        }
    }

    pub fn handle(&self, store: &Store) -> Result<PooledTransactions, StoreError> {
        let txs = self
            .transaction_hashes
            .iter()
            .map(|hash| Self::get_p2p_transaction(hash, store))
            // Return an error in case anything failed.
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            // As per the spec, Nones are perfectly acceptable, for example if a transaction was
            // taken out of the mempool due to payload building after being advertised.
            .flatten()
            .collect();

        // TODO: add getting of the blob bundle, as we'll implement this as a p2p transaction.
        Ok(PooledTransactions {
            id: self.id,
            pooled_transactions: txs,
        })
    }

    /// Gets a p2p transaction given a hash.
    fn get_p2p_transaction(
        hash: &H256,
        store: &Store,
    ) -> Result<Option<P2PTransaction>, StoreError> {
        let Some(tx) = store.get_transaction_by_hash(*hash)? else {
            return Ok(None);
        };
        let result = match tx {
            Transaction::LegacyTransaction(itx) => P2PTransaction::LegacyTransaction(itx),
            Transaction::EIP2930Transaction(itx) => P2PTransaction::EIP2930Transaction(itx),
            Transaction::EIP1559Transaction(itx) => P2PTransaction::EIP1559Transaction(itx),
            Transaction::EIP4844Transaction(itx) => {
                let Some(bundle) = store.get_blobs_bundle_from_pool(*hash)? else {
                    return Err(StoreError::Custom(format!(
                        "Blob transaction present without its bundle: hash {}",
                        hash
                    )));
                };

                P2PTransaction::WrappedEIP4844Transaction(WrappedEIP4844Transaction {
                    tx: itx,
                    blobs_bundle: bundle,
                })
            }
            Transaction::PrivilegedL2Transaction(itx) => {
                P2PTransaction::PrivilegedL2Transaction(itx)
            }
        };

        Ok(Some(result))
    }
}

impl RLPxMessage for GetPooledTransactions {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.transaction_hashes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (transaction_hashes, _): (Vec<H256>, _) = decoder.decode_field("transactionHashes")?;

        Ok(Self::new(id, transaction_hashes))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#pooledtransactions-0x0a
#[derive(Debug)]
pub(crate) struct PooledTransactions {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    id: u64,
    pooled_transactions: Vec<P2PTransaction>,
}

impl PooledTransactions {
    pub fn new(id: u64, pooled_transactions: Vec<P2PTransaction>) -> Self {
        Self {
            pooled_transactions,
            id,
        }
    }

    /// Saves every incoming pooled transaction to the mempool.
    pub fn handle(&self, store: &Store) -> Result<(), MempoolError> {
        for tx in &self.pooled_transactions {
            if let P2PTransaction::WrappedEIP4844Transaction(itx) = tx.clone() {
                mempool::add_blob_transaction(itx.tx, itx.blobs_bundle, store)?;
            } else {
                let regular_tx = tx
                    .clone()
                    .try_into()
                    .map_err(|error| MempoolError::StoreError(StoreError::Custom(error)))?;
                mempool::add_transaction(regular_tx, store)?;
            }
        }
        Ok(())
    }
}

impl RLPxMessage for PooledTransactions {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.pooled_transactions)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (pooled_transactions, _): (Vec<P2PTransaction>, _) =
            decoder.decode_field("pooledTransactions")?;

        Ok(Self::new(id, pooled_transactions))
    }
}

#[cfg(test)]
mod tests {
    use ethrex_core::{types::P2PTransaction, H256};

    use crate::rlpx::{
        eth::transactions::{GetPooledTransactions, PooledTransactions},
        message::RLPxMessage,
    };

    #[test]
    fn get_pooled_transactions_empty_message() {
        let transaction_hashes = vec![];
        let get_pooled_transactions = GetPooledTransactions::new(1, transaction_hashes.clone());

        let mut buf = Vec::new();
        get_pooled_transactions.encode(&mut buf).unwrap();

        let decoded = GetPooledTransactions::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.transaction_hashes, transaction_hashes);
    }

    #[test]
    fn get_pooled_transactions_not_empty_message() {
        let transaction_hashes = vec![
            H256::from_low_u64_be(1),
            H256::from_low_u64_be(2),
            H256::from_low_u64_be(3),
        ];
        let get_pooled_transactions = GetPooledTransactions::new(1, transaction_hashes.clone());

        let mut buf = Vec::new();
        get_pooled_transactions.encode(&mut buf).unwrap();

        let decoded = GetPooledTransactions::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.transaction_hashes, transaction_hashes);
    }

    #[test]
    fn pooled_transactions_of_one_type() {
        let transaction1 = P2PTransaction::LegacyTransaction(Default::default());
        let pooled_transactions = vec![transaction1.clone()];
        let pooled_transactions = PooledTransactions::new(1, pooled_transactions);

        let mut buf = Vec::new();
        pooled_transactions.encode(&mut buf).unwrap();
        let decoded = PooledTransactions::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.pooled_transactions, vec![transaction1]);
    }
}

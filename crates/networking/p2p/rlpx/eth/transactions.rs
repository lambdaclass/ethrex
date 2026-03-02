use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use crate::types::Node;
use bytes::BufMut;
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::MempoolError;
use ethrex_common::types::Fork;
use ethrex_common::types::P2PTransaction;
use ethrex_common::types::WrappedEIP4844Transaction;
use ethrex_common::{H256, types::Transaction};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::error::StoreError;
use tracing::debug;

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
// Broadcast message
#[derive(Debug, Clone)]
pub struct Transactions {
    pub transactions: Vec<Transaction>,
}

impl Transactions {
    pub fn new(transactions: Vec<Transaction>) -> Self {
        Self { transactions }
    }
}

impl RLPxMessage for Transactions {
    const CODE: u8 = 0x02;
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
            decoder = updated_decoder;
            transactions.push(tx);
        }
        Ok(Self::new(transactions))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x08
// Broadcast message
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NewPooledTransactionHashes {
    transaction_types: Bytes,
    transaction_sizes: Vec<usize>,
    pub transaction_hashes: Vec<H256>,
}

impl NewPooledTransactionHashes {
    pub fn new(
        transactions: Vec<Transaction>,
        blockchain: &Blockchain,
    ) -> Result<Self, StoreError> {
        let transactions_len = transactions.len();
        let mut transaction_types = Vec::with_capacity(transactions_len);
        let mut transaction_sizes = Vec::with_capacity(transactions_len);
        let mut transaction_hashes = Vec::with_capacity(transactions_len);
        for transaction in transactions {
            let transaction_type = transaction.tx_type();
            transaction_types.push(transaction_type as u8);
            let transaction_hash = transaction.hash();
            transaction_hashes.push(transaction_hash);
            // size is defined as the len of the canonical encoding of the transaction
            // as it would appear in a PooledTransactions response.
            // https://eips.ethereum.org/EIPS/eip-2718
            let transaction_size = match transaction {
                // Blob transactions use the network (wrapped) representation
                // which includes the blobs bundle.
                // https://eips.ethereum.org/EIPS/eip-4844#networking
                Transaction::EIP4844Transaction(eip4844_tx) => {
                    let tx_blobs_bundle = blockchain
                        .mempool
                        .get_blobs_bundle(transaction_hash)?
                        .unwrap_or_default();
                    let p2p_tx =
                        P2PTransaction::EIP4844TransactionWithBlobs(WrappedEIP4844Transaction {
                            tx: eip4844_tx,
                            wrapper_version: (tx_blobs_bundle.version != 0)
                                .then_some(tx_blobs_bundle.version),
                            blobs_bundle: tx_blobs_bundle,
                        });
                    p2p_tx.encode_canonical_to_vec().len()
                }
                _ => transaction.encode_canonical_to_vec().len(),
            };
            transaction_sizes.push(transaction_size);
        }
        Ok(Self {
            transaction_types: transaction_types.into(),
            transaction_sizes,
            transaction_hashes,
        })
    }

    pub fn get_transactions_to_request(
        &self,
        blockchain: &Blockchain,
    ) -> Result<Vec<H256>, StoreError> {
        blockchain
            .mempool
            .filter_unknown_transactions(&self.transaction_hashes)
    }
}

impl RLPxMessage for NewPooledTransactionHashes {
    const CODE: u8 = 0x08;
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
#[derive(Debug, Clone)]
pub struct GetPooledTransactions {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub transaction_hashes: Vec<H256>,
}

impl GetPooledTransactions {
    pub fn new(id: u64, transaction_hashes: Vec<H256>) -> Self {
        Self {
            transaction_hashes,
            id,
        }
    }

    pub fn handle(&self, blockchain: &Blockchain) -> Result<PooledTransactions, StoreError> {
        // TODO(#1615): get transactions in batch instead of iterating over them.
        let txs = self
            .transaction_hashes
            .iter()
            // As per the spec, skipping unavailable transactions is perfectly acceptable,
            // for example if a transaction was taken out of the mempool due to payload
            // building after being advertised.
            .filter_map(|hash| blockchain.get_p2p_transaction_by_hash(hash).ok())
            .collect::<Vec<_>>();

        Ok(PooledTransactions {
            id: self.id,
            pooled_transactions: txs,
        })
    }
}

impl RLPxMessage for GetPooledTransactions {
    const CODE: u8 = 0x09;
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
#[derive(Debug, Clone)]
pub struct PooledTransactions {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub pooled_transactions: Vec<P2PTransaction>,
}

impl PooledTransactions {
    pub fn new(id: u64, pooled_transactions: Vec<P2PTransaction>) -> Self {
        Self {
            pooled_transactions,
            id,
        }
    }

    /// validates if the received TXs match the request
    pub fn validate_requested(
        &self,
        requested: &NewPooledTransactionHashes,
        fork: Fork,
    ) -> Result<(), MempoolError> {
        for tx in &self.pooled_transactions {
            if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx {
                itx.blobs_bundle.validate_cheap(&itx.tx, fork)?;
            }
            let tx_hash = tx.compute_hash();
            let Some(pos) = requested
                .transaction_hashes
                .iter()
                .position(|&hash| hash == tx_hash)
            else {
                return Err(MempoolError::RequestedPooledTxNotFound);
            };

            let expected_type = requested.transaction_types[pos];
            let expected_size = requested.transaction_sizes[pos];
            if tx.tx_type() as u8 != expected_type {
                return Err(MempoolError::InvalidPooledTxType(expected_type));
            }
            let tx_size = tx.encode_canonical_to_vec().len();
            if tx_size != expected_size {
                return Err(MempoolError::InvalidPooledTxSize);
            }
        }
        Ok(())
    }

    /// Saves every incoming pooled transaction to the mempool.
    pub async fn handle(
        self,
        node: &Node,
        blockchain: &Blockchain,
        is_l2_mode: bool,
    ) -> Result<(), MempoolError> {
        for tx in self.pooled_transactions {
            if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx {
                if is_l2_mode {
                    debug!(
                        peer=%node,
                        "Rejecting blob transaction in L2 mode - blob transactions are not supported in L2",
                    );
                    continue;
                }
                if let Err(e) = blockchain
                    .add_blob_transaction_to_pool(itx.tx, itx.blobs_bundle)
                    .await
                {
                    if matches!(e, MempoolError::BlobsBundleError(_)) {
                        return Err(e);
                    }
                    debug!(
                        peer=%node,
                        error=%e,
                        "Error adding transaction"
                    );
                    continue;
                }
            } else {
                let regular_tx = tx
                    .try_into()
                    .map_err(|error| MempoolError::StoreError(StoreError::Custom(error)))?;
                if let Err(e) = blockchain.add_transaction_to_pool(regular_tx).await {
                    debug!(
                        peer=%node,
                        error=%e,
                        "Error adding transaction"
                    );
                    continue;
                }
            }
        }
        Ok(())
    }
}

impl RLPxMessage for PooledTransactions {
    const CODE: u8 = 0x0A;
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
    use ethrex_common::{H256, types::P2PTransaction};

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

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use ethrex_blockchain::{error::MempoolError, mempool};
use ethrex_core::{types::P2PTransaction, H256, H512};
use ethrex_storage::{error::StoreError, Store};
use tokio::sync::Mutex;
use tracing::warn;

use crate::rlpx::{
    error::RLPxError,
    eth::transactions::{NewPooledTransactionHashes, PooledTransactions},
};

const STALE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub struct TransactionRequest {
    pub transaction_hashes: Vec<H256>,
    pub transaction_types: Vec<u8>,
    pub transaction_sizes: Vec<usize>,
    pub timestamp: Instant,
}

impl TransactionRequest {
    pub fn new(
        transaction_hashes: Vec<H256>,
        transaction_types: Vec<u8>,
        transaction_sizes: Vec<usize>,
    ) -> Self {
        Self {
            transaction_hashes,
            transaction_types,
            transaction_sizes,
            timestamp: Instant::now(),
        }
    }
    fn is_stale(&self, time: Instant) -> bool {
        self.timestamp + STALE_REQUEST_TIMEOUT < time
    }
}

/// Keeps only transactions from the received message that are neither in the mempool nor already requested.  
/// Adds unknown transactions to `global_requested_transactions` to prevent re-requesting them.  
/// Returns a new `TransactionRequest`.  
pub async fn get_new_request_from_msg(
    msg: NewPooledTransactionHashes,
    global_requested_transactions: &Arc<Mutex<HashSet<H256>>>,
    storage: &Store,
) -> Result<Option<TransactionRequest>, RLPxError> {
    let mut global_requested_transactions = global_requested_transactions.lock().await;
    let unknown_for_mempool = storage.filter_unknown_transactions(&msg.transaction_hashes)?;
    let mut unknown_tx_hashes = vec![];
    let mut unknown_tx_types = vec![];
    let mut unknown_tx_sizes = vec![];
    for (index, hash) in msg.transaction_hashes.iter().enumerate() {
        if unknown_for_mempool.contains(hash) && !global_requested_transactions.contains(hash) {
            unknown_tx_hashes.push(*hash);
            unknown_tx_types.push(msg.transaction_types[index]);
            unknown_tx_sizes.push(msg.transaction_sizes[index]);
            global_requested_transactions.insert(*hash);
        }
    }
    if !unknown_tx_hashes.is_empty() {
        // All txs already known
        return Ok(None);
    }
    Ok(Some(TransactionRequest::new(
        unknown_tx_hashes,
        unknown_tx_types,
        unknown_tx_sizes,
    )))
}

/// Removes stale transaction requests from `peer_pending_requests`.  
/// If stale requests exist, their transaction hashes are removed from `global_requested_transactions`.
pub async fn remove_stale_requests(
    global_requested_transactions: &Arc<Mutex<HashSet<H256>>>,
    peer_pending_requests: &mut HashMap<u64, TransactionRequest>,
) {
    let time = Instant::now();

    // Collect stale request IDs first (no lock needed yet)
    let stale_requests: Vec<u64> = peer_pending_requests
        .iter()
        .filter_map(|(id, req)| req.is_stale(time).then_some(*id))
        .collect();

    if stale_requests.is_empty() {
        return;
    }

    // Remove stale requests and update global transactions
    let mut global_requested_transactions = global_requested_transactions.lock().await;

    for request_id in &stale_requests {
        if let Some(stale_request) = peer_pending_requests.remove(request_id) {
            for hash in &stale_request.transaction_hashes {
                global_requested_transactions.remove(hash);
            }
        }
    }
}

pub async fn remove_peer_pending_requests(
    global_requested_transactions: &Arc<Mutex<HashSet<H256>>>,
    peer_pending_requests: &mut HashMap<u64, TransactionRequest>,
) {
    let mut global_requested_transactions = global_requested_transactions.lock().await;
    for (_, request) in peer_pending_requests.iter() {
        for hash in &request.transaction_hashes {
            global_requested_transactions.remove(hash);
        }
    }
}

/// Validates the received transactions against the previous request.
/// Saves every incoming pooled transaction to the mempool.
/// Removes all requested transactions from `global_requested_transactions`.
pub async fn handle_response(
    response: PooledTransactions,
    store: &Store,
    remote_node_id: H512,
    global_requested_transactions: &Arc<Mutex<HashSet<H256>>>,
    peer_pending_requests: &mut HashMap<u64, TransactionRequest>,
) -> Result<(), RLPxError> {
    let request = peer_pending_requests.get(&response.id);
    if request.is_none() {
        // Unknown id. It may be a request from a previous run. Ignoring msg...
        return Ok(());
    }

    validate_response(&response, request.unwrap())?;
    for tx in response.pooled_transactions {
        if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx {
            if let Err(e) = mempool::add_blob_transaction(itx.tx, itx.blobs_bundle, store) {
                warn!(
                    "Error adding transaction from peer {}: {}",
                    remote_node_id, e
                );
            }
        } else {
            let regular_tx = tx
                .try_into()
                .map_err(|error| MempoolError::StoreError(StoreError::Custom(error)))?;
            if let Err(e) = mempool::add_transaction(regular_tx, store) {
                warn!(
                    "Error adding transaction from peer {}: {}",
                    remote_node_id, e
                );
            }
        }
    }
    // The received txs were added to the mempool, it's safe to remove them from `global_requested_transactions`.
    let mut global_requested_transactions = global_requested_transactions.lock().await;
    let request = peer_pending_requests.remove(&response.id).unwrap();
    for hash in request.transaction_hashes {
        global_requested_transactions.remove(&hash);
    }
    Ok(())
}

// Matches the received message with the request made.
// Ensures the received txs are in order.
// Ensures the received types and sizes matches the announced ones.
// Some of the requested txs may not be responded.
fn validate_response(
    response: &PooledTransactions,
    request: &TransactionRequest,
) -> Result<(), RLPxError> {
    let mut last_index: i32 = -1;
    for received_tx in &response.pooled_transactions {
        let received_tx_hash = received_tx.compute_hash();
        let received_tx_size = 1 + received_tx.tx_data().len();
        let received_tx_type = received_tx.tx_type() as u8;

        if let Some(index) = request
            .transaction_hashes
            .iter()
            .position(|x| *x == received_tx_hash)
        {
            // Ensure the txs are in order.
            // With this we also avoid repeated transactions.
            if index as i32 <= last_index {
                return Err(RLPxError::BadRequest(
                    "Invalid order in PoolTransactions message.".to_string(),
                ));
            }
            if received_tx_type != request.transaction_types[index] {
                return Err(RLPxError::BadRequest(
                    "Invalid type in PoolTransactions message.".to_string(),
                ));
            }
            if received_tx_size != request.transaction_sizes[index] {
                return Err(RLPxError::BadRequest(
                    "Invalid size in PoolTransactions message.".to_string(),
                ));
            }
            last_index = index as i32;
        } else {
            return Err(RLPxError::BadRequest(
                "Transaction not requested received in PoolTransactions message".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_core::{
        types::{EIP2930Transaction, LegacyTransaction},
        H256,
    };
    use rand::random;
    use std::thread::sleep;

    #[allow(clippy::type_complexity)]
    fn setup() -> (
        Arc<Mutex<HashSet<H256>>>,
        HashMap<u64, TransactionRequest>,
        Vec<H256>,
        u64,
    ) {
        let mut global_requested_transactions = HashSet::new();
        let mut peer_pending_requests = HashMap::new();
        let tx_1 = H256::random();
        let tx_2 = H256::random();
        let transactions = vec![tx_1, tx_2];
        global_requested_transactions.insert(tx_1);
        global_requested_transactions.insert(tx_2);
        let request = TransactionRequest::new(transactions.clone(), vec![], vec![]);
        let request_id = random();
        peer_pending_requests.insert(request_id, request);
        (
            Arc::new(Mutex::new(global_requested_transactions)),
            peer_pending_requests,
            transactions,
            request_id,
        )
    }

    #[test]
    fn test_transaction_request_staleness() {
        let request = TransactionRequest::new(vec![], vec![], vec![]);
        sleep(STALE_REQUEST_TIMEOUT + Duration::from_secs(1));
        assert!(request.is_stale(Instant::now()));
    }

    #[tokio::test]
    async fn test_remove_peer_request() {
        let (global_requested_transactions, mut pending_requests, tx_hashes, _request_id) = setup();

        remove_peer_pending_requests(&global_requested_transactions, &mut pending_requests).await;

        assert!(!global_requested_transactions
            .lock()
            .await
            .contains(&tx_hashes[0]));
        assert!(!global_requested_transactions
            .lock()
            .await
            .contains(&tx_hashes[1]));
    }

    #[tokio::test]
    async fn test_remove_stale_requests() {
        let (
            global_requested_transactions,
            mut pending_requests,
            stale_tx_hashes,
            stale_request_id,
        ) = setup();

        sleep(STALE_REQUEST_TIMEOUT + Duration::from_secs(1));

        let (_, mut fresh_pending_requests, fresh_tx_hashes, fresh_request_id) = setup();

        // Insert the fresh request along with the stale one.
        pending_requests.insert(
            fresh_request_id,
            fresh_pending_requests.remove(&fresh_request_id).unwrap(),
        );
        // Insert the fresh tx hashes along with the stale ones.
        {
            let mut global_requested_transactions = global_requested_transactions.lock().await;
            global_requested_transactions.insert(fresh_tx_hashes[0]);
            global_requested_transactions.insert(fresh_tx_hashes[1]);
        }

        remove_stale_requests(&global_requested_transactions, &mut pending_requests).await;

        let global_requested_transactions = global_requested_transactions.lock().await;

        assert_eq!(pending_requests.len(), 1);
        assert_eq!(pending_requests.get(&stale_request_id), None);
        assert!(pending_requests.contains_key(&fresh_request_id));
        assert!(!global_requested_transactions.contains(&stale_tx_hashes[0]));
        assert!(!global_requested_transactions.contains(&stale_tx_hashes[1]));
        assert!(global_requested_transactions.contains(&fresh_tx_hashes[0]));
        assert!(global_requested_transactions.contains(&fresh_tx_hashes[1]));
    }

    fn setup_validation() -> (P2PTransaction, P2PTransaction, PooledTransactions) {
        let tx1 = LegacyTransaction {
            data: vec![0x01, 0x02].into(),
            ..Default::default()
        };
        let tx1 = P2PTransaction::LegacyTransaction(tx1);

        let tx2 = EIP2930Transaction {
            data: vec![0x03, 0x04].into(),
            ..Default::default()
        };
        let tx2 = P2PTransaction::EIP2930Transaction(tx2);

        let pool_msg = PooledTransactions {
            id: 0,
            pooled_transactions: vec![tx1.clone(), tx2.clone()],
        };

        (tx1, tx2, pool_msg)
    }

    #[test]
    fn test_validate_successful() {
        let (tx1, tx2, pool_msg) = setup_validation();
        let request = TransactionRequest {
            transaction_hashes: vec![tx1.compute_hash(), tx2.compute_hash()],
            transaction_sizes: vec![3, 3], // 1 + tx_data.len()
            transaction_types: vec![0, 1],
            timestamp: Instant::now(),
        };

        let result = validate_response(&pool_msg, &request);

        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_invalid_order() {
        let (tx1, tx2, pool_msg) = setup_validation();
        let request = TransactionRequest {
            transaction_hashes: vec![tx2.compute_hash(), tx1.compute_hash()],
            transaction_sizes: vec![3, 3], // 1 + tx_data.len()
            transaction_types: vec![1, 0],
            timestamp: Instant::now(),
        };

        let result = validate_response(&pool_msg, &request);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Bad Request: Invalid order in PoolTransactions message.".to_string()
        );
    }

    #[test]
    fn test_validate_invalid_type() {
        let (tx1, tx2, pool_msg) = setup_validation();

        let request = TransactionRequest {
            transaction_hashes: vec![tx1.compute_hash(), tx2.compute_hash()],
            transaction_sizes: vec![3, 3], // 1 + tx_data.len()
            transaction_types: vec![0, 2],
            timestamp: Instant::now(),
        };

        let result = validate_response(&pool_msg, &request);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Bad Request: Invalid type in PoolTransactions message.".to_string()
        );
    }

    #[test]
    fn test_validate_invalid_size() {
        let (tx1, tx2, pool_msg) = setup_validation();

        let request = TransactionRequest {
            transaction_hashes: vec![tx1.compute_hash(), tx2.compute_hash()],
            transaction_sizes: vec![1, 3], // 1 + tx_data.len()
            transaction_types: vec![0, 2],
            timestamp: Instant::now(),
        };

        let result = validate_response(&pool_msg, &request);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Bad Request: Invalid size in PoolTransactions message.".to_string()
        );
    }

    #[test]
    fn test_validate_transaction_not_requested() {
        let (tx1, _, pool_msg) = setup_validation();

        let request = TransactionRequest {
            transaction_hashes: vec![tx1.compute_hash()],
            transaction_sizes: vec![3], // 1 + tx_data.len()
            transaction_types: vec![0],
            timestamp: Instant::now(),
        };

        let result = validate_response(&pool_msg, &request);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Bad Request: Transaction not requested received in PoolTransactions message"
                .to_string()
        );
    }
}

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use ethereum_types::H256;
use ethrex_core::H512;

const STALE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct TransactionRequest {
    pub id: u64,
    pub transaction_hashes: Vec<H256>,
    pub transaction_types: Vec<u8>,
    pub transaction_sizes: Vec<usize>,
    pub timestamp: Instant,
}

impl TransactionRequest {
    pub fn new(
        id: u64,
        transaction_hashes: Vec<H256>,
        transaction_types: Vec<u8>,
        transaction_sizes: Vec<usize>,
    ) -> Self {
        Self {
            id,
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
#[derive(Debug)]
pub(crate) struct PendingRequests {
    pending_transactions: HashSet<H256>,
    requests_by_node_id: HashMap<H512, Vec<TransactionRequest>>,
}

impl PendingRequests {
    pub fn new() -> Self {
        Self {
            pending_transactions: HashSet::new(),
            requests_by_node_id: HashMap::new(),
        }
    }
    pub fn contains_txs(&self, tx_hash: &H256) -> bool {
        self.pending_transactions.contains(tx_hash)
    }
    pub fn store_request(&mut self, remote_node_id: H512, new_request: TransactionRequest) {
        for tx_hash in new_request.transaction_hashes.clone() {
            self.pending_transactions.insert(tx_hash);
        }
        if let Some(requests) = self.requests_by_node_id.get_mut(&remote_node_id) {
            requests.push(new_request);
        } else {
            self.requests_by_node_id
                .insert(remote_node_id, vec![new_request]);
        }
    }

    pub fn get_pending_request(
        &self,
        remote_node_id: &H512,
        request_id: u64,
    ) -> Option<TransactionRequest> {
        if let Some(peer_requests) = self.requests_by_node_id.get(remote_node_id) {
            for request in peer_requests {
                if request.id == request_id {
                    return Some(request.clone());
                }
            }
        }
        None
    }

    pub fn remove_pending_request(&mut self, remote_node_id: &H512, request_id: u64) {
        if let Some(peer_requests) = self.requests_by_node_id.get_mut(remote_node_id) {
            if let Some(index) = peer_requests
                .iter()
                .position(|request| request.id == request_id)
            {
                let request = peer_requests.remove(index);
                for hash in request.transaction_hashes {
                    self.pending_transactions.remove(&hash);
                }
            }
        }
    }

    pub fn remove_peer_requests(&mut self, remote_node_id: &H512) {
        if let Some(peer_requests) = self.requests_by_node_id.remove(remote_node_id) {
            for request in peer_requests {
                for hash in request.transaction_hashes {
                    self.pending_transactions.remove(&hash);
                }
            }
        }
    }

    pub fn remove_stale_requests(&mut self, remote_node_id: &H512) {
        if let Some(peer_requests) = self.requests_by_node_id.get_mut(remote_node_id) {
            let time = Instant::now();
            for request in peer_requests.iter() {
                if request.is_stale(time) {
                    for hash in &request.transaction_hashes {
                        self.pending_transactions.remove(hash);
                    }
                }
            }
            peer_requests.retain(|req| !req.is_stale(time));
        }
    }
}

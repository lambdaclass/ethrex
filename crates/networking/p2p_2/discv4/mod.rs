use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, atomic::AtomicU64},
};

use ethrex_common::H256;
use tokio::sync::Mutex;
use tracing::info;

use crate::types::Node;

pub mod messages;
pub mod server;
pub mod side_car;

// pub type Kademlia = Arc<Mutex<HashMap<H256, Node>>>;

#[derive(Debug, Clone)]
pub struct Kademlia {
    pub table: Arc<Mutex<HashMap<H256, Node>>>,
    pub peers: Arc<Mutex<HashSet<H256>>>,
    pub already_tried_peers: Arc<Mutex<HashSet<H256>>>, // pub number_of_peers: Arc<Mutex<u64>>
}

impl Kademlia {
    pub fn new() -> Self {
        Self {
            table: Arc::new(Mutex::new(HashMap::new())),
            // number_of_peers: Arc::new(Mutex::new(0))
            peers: Arc::new(Mutex::new(HashSet::default())),
            already_tried_peers: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn number_of_peers(&self) -> u64 {
        let peers = self.peers.lock().await;
        peers.len() as u64
    }

    pub async fn number_of_tried_peers(&self) -> u64 {
        let peers = self.already_tried_peers.lock().await;
        peers.len() as u64
    }

    pub async fn set_connected_peer(&mut self, node_id: H256) {
        info!("New peer connected");
        self.peers.lock().await.insert(node_id);
        // let mut number_of_peers = self.number_of_peers.lock().await;
        // *number_of_peers += 1;
    }
}

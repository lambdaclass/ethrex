use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use ethrex_common::H256;
use tokio::sync::Mutex;
use tracing::info;

use crate::types::Node;

#[derive(Debug, Clone)]
pub struct Contact {
    pub node: Node,
    pub n_find_node_sent: u64,
    // This contact failed to respond our Ping.
    pub disposable: bool,
    // Set to true after we send a successful ENRResponse to it.
    pub knows_us: bool,
}

impl From<Node> for Contact {
    fn from(node: Node) -> Self {
        Self {
            node,
            n_find_node_sent: 0,
            disposable: false,
            knows_us: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Kademlia {
    pub table: Arc<Mutex<HashMap<H256, Contact>>>,
    pub peers: Arc<Mutex<HashSet<H256>>>,
    pub already_tried_peers: Arc<Mutex<HashSet<H256>>>,
    pub discarded_contacts: Arc<Mutex<HashSet<H256>>>,
    pub discovered_mainnet_peers: Arc<Mutex<HashSet<H256>>>,
}

impl Kademlia {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn number_of_contacts(&self) -> u64 {
        let contacts = self.table.lock().await;
        contacts.len() as u64
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
    }
}

impl Default for Kademlia {
    fn default() -> Self {
        Self {
            table: Arc::new(Mutex::new(HashMap::new())),
            peers: Arc::new(Mutex::new(HashSet::default())),
            already_tried_peers: Arc::new(Mutex::new(HashSet::new())),
            discarded_contacts: Arc::new(Mutex::new(HashSet::new())),
            discovered_mainnet_peers: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

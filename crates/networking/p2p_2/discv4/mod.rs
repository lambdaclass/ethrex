use std::{collections::HashMap, sync::Arc};

use ethrex_common::H256;
use tokio::sync::Mutex;

use crate::types::Node;

pub mod messages;
pub mod metrics;
pub mod server;
pub mod side_car;

pub type Kademlia = Arc<Mutex<HashMap<H256, Node>>>;

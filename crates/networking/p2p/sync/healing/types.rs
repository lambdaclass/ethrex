use std::collections::HashMap;

use crate::snap::mpt_stubs::{Nibbles, Node};

/// Entry in the healing queue tracking nodes waiting for children
#[derive(Debug, Clone)]
pub struct HealingQueueEntry {
    pub node: Node,
    pub pending_children_count: usize,
    pub parent_path: Nibbles,
}

/// Type alias for state healing queue
pub type StateHealingQueue = HashMap<Nibbles, HealingQueueEntry>;
